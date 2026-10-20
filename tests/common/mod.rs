use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use typesafers::{Client, RetryPolicy, Value};

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub body_delay: Duration,
    pub disconnect: bool,
}
impl Reply {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: vec![],
            body_delay: Duration::ZERO,
            disconnect: false,
        }
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

#[derive(Debug)]
pub struct Captured {
    pub line: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Server {
    pub url: String,
    count: Arc<AtomicUsize>,
    requests: mpsc::UnboundedReceiver<Captured>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let number = seen.fetch_add(1, Ordering::SeqCst);
                        let reply = replies[number.min(replies.len() - 1)].clone();
                        let tx = tx.clone();
                        connections.spawn(async move {
                            let mut bytes = Vec::new();
                            let header_end = loop {
                                let mut chunk = [0; 4096];
                                let n = socket.read(&mut chunk).await.unwrap();
                                if n == 0 { return; }
                                bytes.extend_from_slice(&chunk[..n]);
                                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") { break pos + 4; }
                            };
                            let text = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                            let mut lines = text.lines();
                            let line = lines.next().unwrap().to_owned();
                            let headers: BTreeMap<_, _> = lines.filter_map(|line| line.split_once(':'))
                                .map(|(k, v)| (k.to_lowercase(), v.trim().to_owned())).collect();
                            let length: usize = headers.get("content-length").map(|v| v.parse().unwrap()).unwrap_or(0);
                            while bytes.len() < header_end + length {
                                let mut chunk = [0; 4096];
                                let n = socket.read(&mut chunk).await.unwrap();
                                if n == 0 { return; }
                                bytes.extend_from_slice(&chunk[..n]);
                            }
                            let _ = tx.send(Captured { line, headers, body: bytes[header_end..header_end + length].to_vec() });
                            if reply.disconnect { return; }
                            let mut response = format!("HTTP/1.1 {} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n", reply.status, reply.body.len());
                            for (name, value) in reply.headers { response.push_str(&format!("{name}: {value}\r\n")); }
                            response.push_str("\r\n");
                            if socket.write_all(response.as_bytes()).await.is_err() { return; }
                            tokio::time::sleep(reply.body_delay).await;
                            let _ = socket.write_all(reply.body.as_bytes()).await;
                        });
                    }
                    Some(result) = connections.join_next() => { result.unwrap(); }
                }
            }
        });
        Self {
            url,
            count,
            requests,
            task,
        }
    }
    pub fn client(&self) -> Client {
        Client::builder()
            .api_key("test-key")
            .base_url(&self.url)
            .model("test-model")
            .retry(RetryPolicy::disabled())
            .build()
            .unwrap()
    }
    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
    pub async fn request(&mut self) -> Captured {
        tokio::time::timeout(Duration::from_secs(2), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

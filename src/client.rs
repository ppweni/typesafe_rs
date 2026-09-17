use std::{fmt, sync::Arc, time::Duration};

use bytes::Bytes;
use reqwest::{
    Method, Url,
    header::{self, HeaderMap, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio::time::Instant;

use crate::{
    API_KEY_ENV, ApiError, BASE_URL_ENV, DEFAULT_BASE_URL, DEFAULT_MODEL, DEFAULT_MODEL_ENV,
    DEFAULT_TIMEOUT, Error, ListModelsResponse, Question, Questions, Response,
    ResponseValidationError, Result, RetryPolicy, SystemOneResponse,
};

const SDK: &str = concat!("typesafe-rs/", env!("CARGO_PKG_VERSION"));

/// A cheaply clonable asynchronous client. Clones share the HTTP connection pool.
/// Requests require a Tokio runtime and may run concurrently through `&Client`.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: Url,
    model: String,
    authorization: HeaderValue,
    headers: HeaderMap,
    timeout: Duration,
    retry: RetryPolicy,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .field("model", &self.inner.model)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Resolve configuration from the `TYPESAFE_*` environment variables.
    pub fn new() -> Result<Self> {
        Self::builder().build()
    }
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Ask named questions about any serializable text, object, or array.
    /// No I/O happens until `send().await`.
    pub fn system_one<S: Serialize>(&self, state: S) -> SystemOneRequest<'_, S> {
        SystemOneRequest {
            client: self,
            state,
            questions: Questions::new(),
            model: None,
            extra_body: Map::new(),
            options: RequestOptions::default(),
        }
    }

    pub fn models(&self) -> Models<'_> {
        Models { client: self }
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Bytes>,
        options: RequestOptions,
    ) -> Result<Response<T>> {
        let timeout = options.timeout.unwrap_or(self.inner.timeout);
        validate_timeout(timeout)?;
        let retry = options.retry.as_ref().unwrap_or(&self.inner.retry);
        retry.validate()?;
        let url = self
            .inner
            .base_url
            .join(path)
            .map_err(|_| Error::Configuration("could not construct endpoint URL".into()))?;
        let endpoint = format!("{method} {url}");
        let mut headers = self.inner.headers.clone();
        headers.extend(options.headers);
        headers.insert(header::AUTHORIZATION, self.inner.authorization.clone());
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(header::USER_AGENT, HeaderValue::from_static(SDK));
        headers.insert("x-typesafe-sdk", HeaderValue::from_static(SDK));
        headers.insert("x-typesafe-runtime", HeaderValue::from_static("rust"));
        if body.is_some() {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }

        let started = Instant::now();
        let mut attempt = 0;
        loop {
            // Explicit zero also prevents a supplied HTTP client's default headers from
            // injecting a false retry count into the first attempt.
            headers.insert("x-typesafe-retry-count", HeaderValue::from(attempt));
            let mut request = self
                .inner
                .http
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .timeout(timeout);
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let result = execute(request, &endpoint).await;
            let error = match result {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            if attempt >= retry.max_retries || !retry.retryable(&error) {
                return Err(error);
            }
            let delay = retry.delay(attempt, &error);
            if retry
                .timeout
                .is_some_and(|budget| started.elapsed().saturating_add(delay) >= budget)
            {
                return Err(error);
            }
            tokio::time::sleep(delay).await;
            // A busy runtime can resume a sleep after the budget is already exhausted.
            if retry
                .timeout
                .is_some_and(|budget| started.elapsed() >= budget)
            {
                return Err(error);
            }
            attempt += 1;
        }
    }
}

/// Explicit values override environment variables; blank environment values are ignored.
#[derive(Default)]
#[must_use]
pub struct ClientBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    headers: HeaderMap,
    timeout: Option<Duration>,
    retry: RetryPolicy,
    http_client: Option<reqwest::Client>,
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder").finish_non_exhaustive()
    }
}

impl ClientBuilder {
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Supply a client for custom proxies, TLS, or pooling. The SDK still applies its
    /// per-attempt timeout. The supplied client's redirect policy remains in effect.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    pub fn build(self) -> Result<Client> {
        let api_key = resolve(self.api_key, API_KEY_ENV, None)
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| Error::Configuration(format!("pass an API key or set {API_KEY_ENV}")))?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| Error::Configuration("API key is not a valid HTTP header value".into()))?;
        authorization.set_sensitive(true);

        let base_url = resolve(self.base_url, BASE_URL_ENV, Some(DEFAULT_BASE_URL)).unwrap();
        let mut base_url = Url::parse(&base_url)
            .map_err(|_| Error::Configuration("base URL must be an absolute HTTP(S) URL".into()))?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(Error::Configuration(
                "base URL must be HTTP(S), without credentials, query, or fragment".into(),
            ));
        }
        // Preserve gateway path prefixes when joining relative endpoint paths.
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let model = resolve(self.model, DEFAULT_MODEL_ENV, Some(DEFAULT_MODEL)).unwrap();
        if model.trim().is_empty() {
            return Err(Error::Configuration("model must not be empty".into()));
        }
        let timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT);
        validate_timeout(timeout)?;
        self.retry.validate()?;
        let http = match self.http_client {
            Some(client) => client,
            None => reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        };
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base_url,
                model,
                authorization,
                headers: self.headers,
                timeout,
                retry: self.retry,
            }),
        })
    }
}

#[derive(Default)]
struct RequestOptions {
    timeout: Option<Duration>,
    retry: Option<RetryPolicy>,
    headers: HeaderMap,
}

#[must_use = "requests are only executed by send().await"]
pub struct SystemOneRequest<'a, S> {
    client: &'a Client,
    state: S,
    questions: Questions,
    model: Option<String>,
    extra_body: Map<String, Value>,
    options: RequestOptions,
}

impl<S: Serialize> SystemOneRequest<'_, S> {
    pub fn question(mut self, name: impl Into<String>, question: impl Into<Question>) -> Self {
        self.questions.insert(name.into(), question.into());
        self
    }
    /// Add named questions, replacing earlier questions with the same name.
    pub fn questions<I, N, Q>(mut self, questions: I) -> Self
    where
        I: IntoIterator<Item = (N, Q)>,
        N: Into<String>,
        Q: Into<Question>,
    {
        self.questions.extend(
            questions
                .into_iter()
                .map(|(name, question)| (name.into(), question.into())),
        );
        self
    }
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.options.retry = Some(retry);
        self
    }
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.options.headers.extend(headers);
        self
    }
    /// Shallow-merge additional body fields after the standard fields. Collisions
    /// override state, model, or questions, matching the Python SDK's escape hatch.
    pub fn extra_body(mut self, body: Map<String, Value>) -> Self {
        self.extra_body.extend(body);
        self
    }

    pub async fn send(self) -> Result<Response<SystemOneResponse>> {
        if self.questions.is_empty() {
            return Err(Error::InvalidInput(
                "at least one question is required".into(),
            ));
        }
        for (name, question) in &self.questions {
            question.validate(name)?;
        }
        let state = serde_json::to_value(self.state)?;
        if !matches!(state, Value::String(_) | Value::Object(_) | Value::Array(_)) {
            return Err(Error::InvalidInput(
                "state must be text, an object, or an array".into(),
            ));
        }
        let model = self.model.as_deref().unwrap_or(&self.client.inner.model);
        if model.trim().is_empty() {
            return Err(Error::InvalidInput("model must not be empty".into()));
        }

        #[derive(Serialize)]
        struct Payload<'a> {
            state: Value,
            model: &'a str,
            questions: Questions,
        }
        let payload = Payload {
            state,
            model,
            questions: self.questions,
        };
        // Encode directly on the normal path. Materialize a body map only for the
        // explicit last-write-wins escape hatch. Bytes clones are O(1) on retries.
        let body = if self.extra_body.is_empty() {
            serde_json::to_vec(&payload)?
        } else {
            let Value::Object(mut body) = serde_json::to_value(payload)? else {
                unreachable!()
            };
            body.extend(self.extra_body);
            serde_json::to_vec(&body)?
        };
        self.client
            .send(
                Method::POST,
                "v1/systemone",
                Some(body.into()),
                self.options,
            )
            .await
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Models<'a> {
    client: &'a Client,
}
impl<'a> Models<'a> {
    pub fn list(&self) -> ModelsRequest<'a> {
        ModelsRequest {
            client: self.client,
            options: RequestOptions::default(),
        }
    }
}

#[must_use = "requests are only executed by send().await"]
pub struct ModelsRequest<'a> {
    client: &'a Client,
    options: RequestOptions,
}
impl ModelsRequest<'_> {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.options.retry = Some(retry);
        self
    }
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.options.headers.extend(headers);
        self
    }
    pub async fn send(self) -> Result<Response<ListModelsResponse>> {
        self.client
            .send(Method::GET, "v1/models", None, self.options)
            .await
    }
}

fn resolve(explicit: Option<String>, name: &str, fallback: Option<&str>) -> Option<String> {
    explicit
        .or_else(|| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| fallback.map(str::to_owned))
}
fn validate_timeout(timeout: Duration) -> Result<()> {
    if timeout.is_zero() {
        return Err(Error::Configuration(
            "request timeout must be positive".into(),
        ));
    }
    Ok(())
}

async fn execute<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
    endpoint: &str,
) -> Result<Response<T>> {
    let response = request.send().await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await?;
    if !status.is_success() {
        return Err(Error::Api(Box::new(ApiError::new(
            status,
            headers,
            body,
            endpoint.into(),
        ))));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(&body);
    let decoded = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| (error.path().to_string(), error.into_inner()))
        .and_then(|data| {
            deserializer
                .end()
                .map(|()| data)
                .map_err(|error| (".".into(), error))
        });
    match decoded {
        Ok(data) => Ok(Response {
            data,
            status,
            headers,
            body,
        }),
        Err((field_path, source)) => Err(Error::ResponseValidation(Box::new(
            ResponseValidationError {
                status,
                headers,
                body,
                endpoint: endpoint.into(),
                field_path,
                source,
            },
        ))),
    }
}

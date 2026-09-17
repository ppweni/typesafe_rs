use crate::{
    api, assess, capture,
    cli::Analysis,
    dashboard,
    engine::{Engine, Window},
    output,
    presentation::Presentation,
};
use anyhow::{Context, Result};
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{
    sync::mpsc,
    task::JoinSet,
    time::{Instant, MissedTickBehavior},
};

pub async fn run(
    name: String,
    window_secs: u64,
    duration_secs: Option<u64>,
    options: Analysis,
) -> Result<()> {
    if options.pretty {
        dashboard::ensure_terminal()?;
    }
    let (client, excluded) = api::configure(options.assess).await?;
    let (tx, mut rx) = mpsc::channel(8192);
    let capture = capture::start(&name, excluded, tx)?;
    let mut presentation = Presentation::new(&name, window_secs, &options)?;
    let mut engine = Engine::new(options.config());
    let mut jobs = JoinSet::new();
    let interval = Duration::from_secs(window_secs);
    let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut redraw = tokio::time::interval(Duration::from_millis(250));
    redraw.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let deadline = async {
        match duration_secs {
            Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds)).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(deadline);
    let mut start = Instant::now();
    if !options.pretty {
        eprintln!(
            "TypeParanoid watching {name}; assessment={}. Ctrl-C stops capture.",
            options.assess
        );
    }
    let capture_result = loop {
        tokio::select! {
            result = &mut shutdown => { result.context("could not receive Ctrl-C")?; break Ok(()); }
            () = &mut deadline => break Ok(()),
            event = rx.recv() => match event {
                Some(Ok(packet)) => { presentation.observe(packet); engine.observe(packet); },
                Some(Err(error)) => break Err(error),
                None => break Err(anyhow::anyhow!("capture thread stopped unexpectedly")),
            },
            _ = ticker.tick() => {
                let window = engine.finish(start.elapsed().as_secs_f64());
                start = Instant::now();
                report(window, &mut presentation, client.as_ref(), &mut jobs);
                presentation.health(&capture);
            },
            _ = redraw.tick(), if options.pretty => { presentation.refresh(&capture, jobs.len())?; },
            input = presentation.input(), if options.pretty => { if input? { break Ok(()); } },
            Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                presentation.assessment(&result.context("assessment task failed")?);
            }
        }
    };
    // Stop accepting new packets, then drain the bounded queue before the final window.
    capture.stop.store(true, Ordering::Relaxed);
    rx.close();
    while let Some(event) = rx.recv().await {
        if let Ok(packet) = event {
            engine.observe(packet);
        }
    }
    // Restore the terminal before printing the final local summary or errors.
    drop(presentation);
    output::window(&engine.finish(start.elapsed().as_secs_f64()), options.json);
    output::capture_health(&capture, options.json);
    // Shutdown must not wait for outstanding inference requests or a blocked BPF read.
    if !jobs.is_empty() {
        eprintln!("Cancelling outstanding assessments at shutdown.");
    }
    jobs.abort_all();
    capture_result
}

fn report(
    window: Window,
    presentation: &mut Presentation,
    client: Option<&typesafe_rs::Client>,
    jobs: &mut JoinSet<assess::Batch>,
) {
    presentation.window(&window);
    if let Some(client) = client
        && !window.candidates.is_empty()
    {
        if jobs.len() >= 2 {
            presentation.skipped(window.number);
            return;
        }
        let client = client.clone();
        jobs.spawn(async move { assess::run(&client, window).await });
    }
}

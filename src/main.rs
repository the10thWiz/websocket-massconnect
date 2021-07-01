use std::{io::{Error, ErrorKind}, sync::mpsc, thread, time::{Duration, Instant}};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::runtime::Builder;
use websocket_lite::{ClientBuilder, Message, Opcode, Result};

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Options {
    #[structopt(short, long, default_value = "1000")]
    max: usize,
    #[structopt(short, long, default_value = "8")]
    threads: usize,
    #[structopt(short, long, default_value = "1")]
    num: usize,
    #[structopt(short, long, default_value = "ws://localhost:8000/echo")]
    address: String,
}

#[derive(Debug, Default)]
struct ClientConnection {
    success: bool,
    time: Duration,
}

fn main() {
    let mut opts: Options = Options::from_args();
    println!("Starting with {:?}", opts);
    let address: &'static str = Box::leak(opts.address.into_boxed_str());

    for _ in 0..opts.num {
        let (tx, rx) = mpsc::channel();

        let num = opts.max / opts.threads;
        opts.max = num * opts.threads;
        let mut rets = Vec::with_capacity(opts.threads);
        for _ in 0..opts.threads {
            let tx_inner = tx.clone();
            rets.push(thread::spawn(move || Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let mut ret = Vec::with_capacity(num);
                let local = tokio::task::LocalSet::new();
                let start = Instant::now();
                for _ in 0..num {
                    ret.push(local.spawn_local(try_connect(address)));
                }
                local.await;
                let time = start.elapsed();
                for c in ret {
                    match c.await {
                        Ok(Ok(duration)) => {
                            tx_inner.send(ClientConnection {
                                success: true,
                                time: duration,
                            }).expect("Send Error");
                        },
                        Ok(Err(e)) => {
                            tx_inner.send(ClientConnection {
                                success: false,
                                time: Duration::from_secs(0),
                            }).expect("Send Error");
                            eprintln!("Error: {:?}", e)
                        },
                        Err(e) => {
                            tx_inner.send(ClientConnection {
                                success: false,
                                time: Duration::from_secs(0),
                            }).expect("Send Error");
                            eprintln!("Join Error: {:?}", e)
                        },
                    }
                }
                time
            })));
        }
        percentiles(rx, opts.max);

        std::thread::sleep(Duration::from_secs(2));
    }
}

const TEXT: &'static [u8] = b"Hello";

async fn try_connect(
    address: &str,
) -> Result<Duration> {
    let start = Instant::now();
    let builder = ClientBuilder::new(address)?;
    let mut client = builder.async_connect_insecure().await?;
    client.
        send(Message::new(Opcode::Text, Bytes::from_static(TEXT))?).await?;
    let (response, _client) = client.into_future().await;
    if let Some(Ok(response)) = response {
        if response.into_data() == TEXT {
            return Ok(start.elapsed());
        }
    }
    Err(Box::new(Error::new(ErrorKind::Other, "Failed")))
}

fn percentiles(rx: mpsc::Receiver<ClientConnection>, max: usize) {
    let mut sorted_times = vec![];
    let mut success = 0;
    let mut count = 0;
    while let Ok(result) = rx.recv() {
        if result.success {
            success += 1;
            sorted_times.push(result.time);
        }
        count += 1;
        if count >= max {
            break;
        }
    }
    sorted_times.sort();

    println!("{}/{} connections succeeded", success, max);
    // Sorted from short to long
    println!("Min: {} ms", sorted_times[0].as_millis());
    println!("50%: {} ms", percentile(&sorted_times, 0.5).as_millis());
    println!("80%: {} ms", percentile(&sorted_times, 0.8).as_millis());
    println!("90%: {} ms", percentile(&sorted_times, 0.9).as_millis());
    println!("95%: {} ms", percentile(&sorted_times, 0.95).as_millis());
    println!("99%: {} ms", percentile(&sorted_times, 0.99).as_millis());
    println!("Max: {} ms", sorted_times[sorted_times.len() - 1].as_millis());
}

fn percentile(vec: &Vec<Duration>, percent: f64) -> Duration {
    vec[(vec.len() as f64 * percent) as usize]
}

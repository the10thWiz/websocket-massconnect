use std::{io::{Error, ErrorKind}, sync::mpsc, thread, time::{Duration, Instant}};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::runtime::Builder;
use websocket_lite::{ClientBuilder, Message, Opcode, Result};

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
/// HTTP & WebSocket Stress Testing tool
///
/// Massconnect attempts to make as many connections as possible, within the limits provided.
struct Options {
    #[structopt(default_value = "ws://localhost:8000/echo")]
    address: String,
    #[structopt(short, long, default_value = "1000")]
    /// Maximum number of connections
    max: usize,
    #[structopt(short, long, default_value = "8")]
    /// How many threads to utilize for connecting
    threads: usize,
    #[structopt(short, long, default_value = "1")]
    /// How many iterations
    num: usize,
    #[structopt(short, long)]
    /// Enable Rocket-style authentication prior to connection
    auth: bool,
    #[structopt(short, long)]
    /// Just do HTTP requests
    http: bool,
    #[structopt(short, long)]
    /// Enable stress mode, which disables statistics and the 2 second pause between iterations
    stress: bool,
}

#[derive(Debug, Default)]
struct ClientConnection {
    success: bool,
    time: Duration,
}

fn main() {
    let mut opts: Options = Options::from_args();
    println!("Starting with {:?}", opts);
    let client: &'static reqwest::Client = Box::leak(Box::new(reqwest::Client::new()));
    let address: &'static str = Box::leak(opts.address.into_boxed_str());
    let auth = opts.auth;
    let http = opts.http;

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
                    ret.push(local.spawn_local(try_connect(address, auth, http, client)));
                }
                local.await;
                let time = start.elapsed();
                for c in ret {
                    match c.await {
                        Ok(Ok(duration)) => {
                            let _ = tx_inner.send(ClientConnection {
                                success: true,
                                time: duration,
                            });
                        },
                        Ok(Err(e)) => {
                            let _ = tx_inner.send(ClientConnection {
                                success: false,
                                time: Duration::from_secs(0),
                            });
                            println!("Error: {:?}", e)
                        },
                        Err(e) => {
                            let _ = tx_inner.send(ClientConnection {
                                success: false,
                                time: Duration::from_secs(0),
                            });
                            println!("Join Error: {:?}", e)
                        },
                    }
                }
                time
            })));
        }
        if !opts.stress {
            percentiles(rx, opts.max);
            std::thread::sleep(Duration::from_secs(2));
        } else {
            drop(rx);
        }

    }
}

const TEXT: &'static [u8] = b"Hello";

async fn try_connect(
    address: &str,
    auth: bool,
    http: bool,
    client: &reqwest::Client
) -> Result<Duration> {
    let start = Instant::now();
    if http {
        if client.get(address).send().await?.text().await? == "Hello, world!" {
            return Ok(start.elapsed());
        }
    } else {
        let builder = if auth {
            let res = client.get(address)
                .send()
                .await?
                .text()
                .await?;
            ClientBuilder::new(&format!("ws://localhost:8000{}", res))?
        } else {
            ClientBuilder::new(address)?
        };
        let mut client = builder.async_connect_insecure().await?;
        client.
            send(Message::new(Opcode::Text, Bytes::from_static(TEXT))?).await?;
        let (response, _client) = client.into_future().await;
        if let Some(Ok(response)) = response {
            if response.into_data() == TEXT {
                return Ok(start.elapsed());
            }
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
/*

/ My first attempt was a failure; since I was instatiating a new http client for every request.

At first glance, it seems like DashMap actually performs better than flurry

# DashMap (10000 c / 6 t)
Min: 1571 ms
50%: 4547 ms
80%: 4677 ms
90%: 4724 ms
95%: 4773 ms
99%: 5441 ms
Max: 5521 ms

Min: 1564 ms
50%: 4567 ms
80%: 4723 ms
90%: 4762 ms
95%: 4803 ms
99%: 4877 ms
Max: 4986 ms

# Flurry
Min: 1980 ms
50%: 4545 ms
80%: 4674 ms
90%: 4765 ms
95%: 4824 ms
99%: 4878 ms
Max: 4950 ms

Min: 2117 ms
50%: 4710 ms
80%: 4816 ms
90%: 4919 ms
95%: 4958 ms
99%: 5026 ms
Max: 5071 ms
*/

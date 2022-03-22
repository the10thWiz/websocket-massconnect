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
    #[structopt(long)]
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

    let mut avg_times = [0; 7];
    for run in 0..opts.num {
        let start = Instant::now();
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
        percentiles(rx, opts.max, &mut avg_times);
        println!("Run {} of {} took: {}s", run + 1, opts.num, start.elapsed().as_secs_f64());
        if !opts.stress {
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    println!("\nAverage times:");
    println!("Min: {} ms", avg_times[0] as f64 / opts.num as f64);
    println!("50%: {} ms", avg_times[1] as f64 / opts.num as f64);
    println!("80%: {} ms", avg_times[2] as f64 / opts.num as f64);
    println!("90%: {} ms", avg_times[3] as f64 / opts.num as f64);
    println!("95%: {} ms", avg_times[4] as f64 / opts.num as f64);
    println!("99%: {} ms", avg_times[5] as f64 / opts.num as f64);
    println!("Max: {} ms", avg_times[6] as f64 / opts.num as f64);
}

const TEXT: &'static [u8] = b"Hello";

async fn try_connect(
    address: &str,
    auth: bool,
    http: bool,
    client: &reqwest::Client,
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

fn percentiles(rx: mpsc::Receiver<ClientConnection>, max: usize, avg_times: &mut [u128; 7]) {
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
    if sorted_times.len() == 0 {
        return;
    }
    sorted_times.sort();

    // Sorted from short to long
    let p0 = sorted_times[0].as_millis();
    let p50 = percentile(&sorted_times, 0.5).as_millis();
    let p80 = percentile(&sorted_times, 0.8).as_millis();
    let p90 = percentile(&sorted_times, 0.9).as_millis();
    let p95 = percentile(&sorted_times, 0.95).as_millis();
    let p99 = percentile(&sorted_times, 0.99).as_millis();
    let p100 = sorted_times[sorted_times.len() - 1].as_millis();
    avg_times[0] += p0;
    avg_times[1] += p50;
    avg_times[2] += p80;
    avg_times[3] += p90;
    avg_times[4] += p95;
    avg_times[5] += p99;
    avg_times[6] += p100;

    println!();
    println!("Min: {} ms", p0);
    println!("50%: {} ms", p50);
    println!("80%: {} ms", p80);
    println!("90%: {} ms", p90);
    println!("95%: {} ms", p95);
    println!("99%: {} ms", p99);
    println!("Max: {} ms", p100);
    println!("{}/{} connections succeeded", success, max);
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

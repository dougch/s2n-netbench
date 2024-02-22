// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use netbench::{
    client::{self, AddressMap},
    multiplex, scenario, trace,
    units::Byte,
    Error, Result,
};
use std::path::PathBuf;
use std::{net::IpAddr, ops::Deref, path::Path, str::FromStr, sync::Arc, time::Duration};

mod alloc;
pub use alloc::Allocator;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum Trace {
    Disabled,
    #[default]
    Throughput,
    Stdio,
}

#[derive(Debug, Parser)]
pub struct Server {
    #[arg(short, long, default_value = "::")]
    pub ip: IpAddr,

    #[arg(short, long, default_value = "4433", env = "PORT")]
    pub port: u16,

    #[arg(long, default_value = "netbench")]
    pub application_protocols: Vec<String>,

    #[arg(long, default_value = "0", env = "SERVER_ID")]
    pub server_id: usize,

    #[arg(long, default_value = "throughput", env = "TRACE")]
    pub trace: Vec<Trace>,

    #[arg(long, env = "TRACE_FILE")]
    pub trace_file: Option<PathBuf>,

    #[arg(long, short = 'V')]
    pub verbose: bool,

    #[arg(long, default_value = "8KiB")]
    pub rx_buffer: Byte,

    #[arg(long, default_value = "8KiB")]
    pub tx_buffer: Byte,

    #[arg(env = "SCENARIO")]
    pub scenario: Scenario,

    #[arg(long)]
    pub nagle: bool,

    #[arg(long, env = "MULTITHREADED")]
    pub multithreaded: Option<Option<bool>>,

    /// Forces multiplex mode for the driver
    ///
    /// Without this, the requirement is inferred based on the scenario
    #[arg(long, env = "MULTIPLEX")]
    multiplex: Option<Option<bool>>,
}

impl Server {
    pub fn runtime(&self) -> tokio::runtime::Runtime {
        let multithreaded = match self.multithreaded {
            Some(Some(v)) => v,
            Some(None) => true,
            None => false,
        };
        if multithreaded {
            tokio::runtime::Builder::new_multi_thread()
        } else {
            tokio::runtime::Builder::new_current_thread()
        }
        .enable_all()
        .build()
        .unwrap()
    }

    pub fn scenario(&self) -> Arc<scenario::Server> {
        let id = self.server_id;
        self.scenario.servers[id].clone()
    }

    pub fn certificate(&self) -> (&Arc<scenario::Certificate>, &Arc<scenario::Certificate>) {
        let id = self.server_id;
        let server = &self.scenario.servers[id];
        let cert = &self.scenario.certificates[server.certificate as usize];
        let private_key = &self.scenario.certificates[server.private_key as usize];
        (cert, private_key)
    }

    pub fn trace(&self) -> impl trace::Trace + Clone {
        traces(
            &self.trace[..],
            &self.trace_file,
            self.verbose,
            &self.scenario.traces,
        )
    }

    pub fn multiplex(&self) -> Option<multiplex::Config> {
        // TODO infer this based on the scenario requirements
        if is_multiplex_enabled(self.multiplex) {
            // TODO load this from the scenario configuration
            Some(multiplex::Config::default())
        } else {
            None
        }
    }
}

#[derive(Debug, Parser)]
pub struct Client {
    #[arg(long, default_value = "netbench")]
    pub application_protocols: Vec<String>,

    #[arg(short, long, default_value = "::", env = "LOCAL_IP")]
    pub local_ip: IpAddr,

    #[arg(long, default_value = "0", env = "CLIENT_ID")]
    pub client_id: usize,

    #[arg(long, default_value = "throughput", env = "TRACE")]
    pub trace: Vec<Trace>,

    #[arg(long, env = "TRACE_FILE")]
    pub trace_file: Option<PathBuf>,

    #[arg(long, short = 'V')]
    pub verbose: bool,

    #[arg(long, default_value = "8KiB")]
    pub rx_buffer: Byte,

    #[arg(long, default_value = "8KiB")]
    pub tx_buffer: Byte,

    #[arg(env = "SCENARIO")]
    pub scenario: Scenario,

    #[arg(long)]
    pub nagle: bool,

    #[arg(long, env = "MULTITHREADED")]
    pub multithreaded: Option<Option<bool>>,

    /// Forces multiplex mode for the driver
    ///
    /// Without this, the requirement is inferred based on the scenario
    #[arg(long, env = "MULTIPLEX")]
    multiplex: Option<Option<bool>>,
}

impl Client {
    pub fn runtime(&self) -> tokio::runtime::Runtime {
        let multithreaded = match self.multithreaded {
            Some(Some(v)) => v,
            Some(None) => true,
            None => false,
        };
        if multithreaded {
            tokio::runtime::Builder::new_multi_thread()
        } else {
            tokio::runtime::Builder::new_current_thread()
        }
        .enable_all()
        .build()
        .unwrap()
    }

    pub fn scenario(&self) -> Arc<scenario::Client> {
        let id = self.client_id;
        self.scenario.clients[id].clone()
    }

    pub fn certificate_authorities(&self) -> impl Iterator<Item = Arc<scenario::Certificate>> + '_ {
        let id = self.client_id;
        let certs = &self.scenario.certificates;
        self.scenario.clients[id]
            .certificate_authorities
            .iter()
            .copied()
            .map(move |ca| certs[ca as usize].clone())
    }

    pub async fn address_map(&self) -> Result<AddressMap> {
        let id = self.client_id as u64;
        AddressMap::new(&self.scenario, id, &mut Resolver).await
    }

    pub fn trace(&self) -> impl trace::Trace + Clone {
        traces(
            &self.trace[..],
            &self.trace_file,
            self.verbose,
            &self.scenario.traces,
        )
    }

    pub fn multiplex(&self) -> Option<multiplex::Config> {
        // TODO infer this based on the scenario requirements
        if is_multiplex_enabled(self.multiplex) {
            // TODO load this from the scenario configuration
            Some(multiplex::Config::default())
        } else {
            None
        }
    }
}

fn is_multiplex_enabled(opt: Option<Option<bool>>) -> bool {
    match opt {
        Some(Some(v)) => v,
        Some(None) => true,
        None => false,
    }
}

struct Resolver;

impl Resolver {
    fn get(&self, key: String) -> Result<String> {
        let host =
            std::env::var(&key).map_err(|_| format!("missing {key} environment variable"))?;
        Ok(host)
    }
}

impl client::Resolver for Resolver {
    fn server(&mut self, id: u64) -> Result<String> {
        self.get(format!("SERVER_{id}"))
    }

    fn router(&mut self, router_id: u64, server_id: u64) -> Result<String> {
        self.get(format!("ROUTER_{router_id}_SERVER_{server_id}"))
    }
}

#[derive(Clone, Debug)]
pub struct Scenario(Arc<scenario::Scenario>);

impl FromStr for Scenario {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let scenario = scenario::Scenario::open(Path::new(s))?;
        Ok(Self(Arc::new(scenario)))
    }
}

impl Deref for Scenario {
    type Target = scenario::Scenario;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

fn traces(
    trace: &[Trace],
    trace_file: &Option<PathBuf>,
    verbose: bool,
    traces: &Arc<Vec<String>>,
) -> impl trace::Trace + Clone {
    let enabled = !trace.iter().any(|v| matches!(v, Trace::Disabled));

    let throughput = if enabled && trace.iter().any(|v| matches!(v, Trace::Throughput)) {
        let trace = trace::Throughput::default();
        trace.reporter(Duration::from_secs(1));
        Some(trace)
    } else {
        None
    };

    let stdio = if enabled && trace.iter().any(|v| matches!(v, Trace::Stdio)) {
        let mut trace = trace::StdioLogger::new(traces.clone());
        trace.verbose(verbose);
        Some(trace)
    } else {
        None
    };

    let trace_file = if let Some(trace_file) = trace_file {
        let trace_file = std::fs::File::create(trace_file).unwrap();
        let trace_file = std::io::BufWriter::new(trace_file);
        let mut trace = trace::FileLogger::with_output(trace_file, traces.clone());
        trace.verbose(verbose);
        Some(trace)
    } else {
        None
    };

    let usdt = trace::Usdt::default();

    (usdt, (throughput, (stdio, trace_file)))
}

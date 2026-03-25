#![forbid(unsafe_code)]

use crate::cmd::Cmd;
use crate::http::http_server::HttpServer;
use log::error;

mod cmd;
mod config;
mod database;
mod http;
mod ip;
mod results;

fn main() -> std::io::Result<()> {
    //parse args
    let cmd = Cmd::parse_args();

    if cmd.download_ipdb {
        ip::update_ipdb(
            "https://raw.githubusercontent.com/librespeed/speedtest-rust/master/country_asn.mmdb",
            "country_asn.mmdb",
        );
        return Ok(());
    }

    if let Some(path) = cmd.generate_sample_result.as_deref() {
        results::telemetry::write_sample_result(path)?;
        return Ok(());
    }

    //init configs & statics
    if let Err(e) = config::init_configs(cmd) {
        error!("{e}");
        std::process::exit(1)
    }

    //init database
    let database = database::init();
    match database {
        Ok(mut database) => {
            let runtime = config::init_runtime();
            match runtime {
                Ok(runtime) => {
                    runtime.block_on(async {
                        let http_server = HttpServer::init().await;
                        match http_server {
                            Ok(mut http_server) => {
                                http_server.listen(&mut database).await;
                            }
                            Err(e) => {
                                error!("{e}");
                                std::process::exit(1)
                            }
                        }
                    });
                }
                Err(e) => {
                    error!("{e}");
                    std::process::exit(1)
                }
            }
        }
        Err(e) => {
            error!("{e}");
            std::process::exit(1)
        }
    }
    Ok(())
}

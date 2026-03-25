use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::FontRef;
use tokio::sync::Mutex;

use crate::config::SERVER_CONFIG;
use crate::database::Database;
use crate::http::request::Request;
use crate::http::response::Response;
use crate::results::telemetry::{draw_result, record_result};

pub async fn telemetry_record_route(
    database: &mut Arc<Mutex<dyn Database + Send>>,
    request: &Request,
) -> Response {
    let server_config = SERVER_CONFIG.get().unwrap();
    match server_config.database_type.as_str() {
        "none" => Response::res_200("Telemetry Disabled."),
        _ => {
            let record_result = record_result(request, database).await;
            match record_result {
                Ok(uuid) => {
                    let response_content = format!("id {}", uuid);
                    Response::res_200(&response_content)
                }
                Err(_) => Response::res_500(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Once};
    use tokio::sync::Mutex;

    use crate::config::{ServerConfig, FONT, SERVER_CONFIG};
    use crate::results::TelemetryData;

    fn init_test_globals() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            SERVER_CONFIG.get_or_init(|| ServerConfig {
                bind_address: "127.0.0.1".to_string(),
                listen_port: 8080,
                worker_threads: serde_json::json!(1),
                base_url: "/backend".to_string(),
                ipinfo_api_key: "".to_string(),
                stats_password: "".to_string(),
                redact_ip_addresses: false,
                result_image_theme: "light".to_string(),
                assets_path: "".to_string(),
                database_type: "memory".to_string(),
                database_hostname: None,
                database_name: None,
                database_username: None,
                database_password: None,
                database_file: None,
                enable_tls: false,
                tls_cert_file: "".to_string(),
                tls_key_file: "".to_string(),
            });
            FONT.get_or_init(|| {
                FontRef::try_from_slice(include_bytes!("../../assets/open-sans.ttf")).unwrap()
            });
        });
    }

    struct MalformedTelemetryDb {
        row: TelemetryData,
    }

    impl Database for MalformedTelemetryDb {
        fn insert(&mut self, _data: TelemetryData) -> std::io::Result<()> {
            Ok(())
        }

        fn fetch_by_uuid(&mut self, _uuid: &str) -> std::io::Result<Option<TelemetryData>> {
            Ok(Some(self.row.clone()))
        }

        fn fetch_last_100(&mut self) -> std::io::Result<Vec<TelemetryData>> {
            Ok(vec![self.row.clone()])
        }
    }

    fn malformed_row() -> TelemetryData {
        TelemetryData {
            ip_address: "203.0.113.42".to_string(),
            isp_info: "{bad json".to_string(),
            extra: "{still bad".to_string(),
            user_agent: "test-agent".to_string(),
            lang: "en-US".to_string(),
            download: "100.0".to_string(),
            upload: "20.0".to_string(),
            ping: "5.0".to_string(),
            jitter: "1.0".to_string(),
            log: "".to_string(),
            uuid: "bad-row".to_string(),
            timestamp: 1_700_000_000,
        }
    }

    #[tokio::test]
    async fn malformed_stored_telemetry_does_not_panic_backend_results() {
        init_test_globals();

        let mut database: Arc<Mutex<dyn Database + Send>> =
            Arc::new(Mutex::new(MalformedTelemetryDb {
                row: malformed_row(),
            }));
        let mut params = HashMap::new();
        params.insert("id".to_string(), "bad-row".to_string());

        let response = show_result_route(&mut database, &params).await;

        assert!(response.data.starts_with(b"HTTP/1.1 200 OK"));
        assert!(response
            .data
            .windows(b"image/jpeg".len())
            .any(|window| window == b"image/jpeg"));
    }
}

pub async fn show_result_route(
    database: &mut Arc<Mutex<dyn Database + Send>>,
    params: &HashMap<String, String>,
) -> Response {
    let result_id = params.get("id");
    match result_id {
        Some(result_id) => {
            let mut db = database.lock().await;
            let fetched_result = db.fetch_by_uuid(result_id);
            match fetched_result {
                Ok(fetched_result) => match fetched_result {
                    Some(fetched_telemetry_data) => {
                        let image = draw_result(&fetched_telemetry_data);
                        drop(fetched_telemetry_data);
                        Response::res_200_img(&image)
                    }
                    None => Response::res_404(),
                },
                Err(_) => Response::res_404(),
            }
        }
        None => Response::res_400(),
    }
}

use crate::{commands, host::DesktopHost};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    capability: String,
    page: String,
    id: u32,
    command: String,
    #[serde(default)]
    args: Value,
}

pub fn initialization_script(origin: &str, capability: &str) -> String {
    include_str!("native_bridge.js")
        .replace("__RF_ORIGIN__", &json!(origin).to_string())
        .replace("__RF_CAPABILITY__", &json!(capability).to_string())
}

pub fn same_origin(url: &str, origin: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .zip(reqwest::Url::parse(origin).ok())
        .is_some_and(|(url, expected)| url.origin() == expected.origin())
}

pub fn parse(
    url: &str,
    body: &str,
    origin: &str,
    capability: &str,
) -> Result<Request, &'static str> {
    if !same_origin(url, origin) || body.len() > 1024 * 1024 {
        return Err("Unauthorized native request");
    }
    let request: Request = serde_json::from_str(body).map_err(|_| "Malformed native request")?;
    if request.capability != capability
        || uuid::Uuid::parse_str(&request.page).is_err()
        || request.id == 0
        || request.command.len() > 80
        || !request.args.is_object()
    {
        return Err("Invalid native request");
    }
    Ok(request)
}

pub async fn dispatch(host: &DesktopHost, request: Request) -> String {
    let response = match commands::dispatch(host, &request.command, request.args).await {
        Ok(value) => json!({ "id": request.id, "page": request.page, "ok": true, "value": value }),
        Err(error) => {
            json!({ "id": request.id, "page": request.page, "ok": false, "error": error })
        }
    };
    format!("window.__rfNativeReply?.({response});")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_commands_require_exact_origin_and_capability() {
        let body = json!({"capability":"secret", "page":uuid::Uuid::new_v4().to_string(),
            "id":1,"command":"get_accounts","args":{}})
        .to_string();
        let origin = "http://127.0.0.1:31234";
        assert!(parse(origin, &body, origin, "secret").is_ok());
        for url in [
            "https://reversedfront.tw",
            "http://127.0.0.1:31235",
            "http://127.0.0.1.evil:31234",
        ] {
            assert!(parse(url, &body, origin, "secret").is_err());
        }
        assert!(parse(origin, &body, origin, "wrong").is_err());
        assert!(parse(origin, "{}", origin, "secret").is_err());
    }
}

use serde_json::{json, Value};
use crate::utils::error::AppError;

pub async fn create_dns_record(
    fqdn: &str, 
    target_ip: &str, 
    proxy: bool,
    custom_token: Option<&str>,
    custom_zone_id: Option<&str>,
) -> Result<(String, String), AppError> {
    let token = match custom_token {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => std::env::var("CLOUDFLARE_API_TOKEN").map_err(|_| AppError::Fatal(anyhow::anyhow!("Missing Token")))?,
    };
    let zone_id = match custom_zone_id {
        Some(z) if !z.trim().is_empty() => z.to_string(),
        _ => std::env::var("CLOUDFLARE_ZONE_ID").map_err(|_| AppError::Fatal(anyhow::anyhow!("Missing Zone ID")))?,
    };

    let url = format!("https://api.cloudflare.com/client/v4/zones/{}/dns_records", zone_id);
    let client = reqwest::Client::new();
    
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&json!({
            "type": "A",
            "name": fqdn,
            "content": target_ip,
            "ttl": 1,
            "proxied": proxy
        }))
        .send()
        .await
        .map_err(|e| AppError::Infrastructure(e.to_string()))?;

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();
    
    if !status.is_success() {
        return Err(AppError::Infrastructure(format!("Cloudflare API error ({}): {}", status, body_text)));
    }

    let body: Value = serde_json::from_str(&body_text).map_err(|e| AppError::Fatal(e.into()))?;
    let record_id = body["result"]["id"].as_str()
        .ok_or_else(|| AppError::Fatal(anyhow::anyhow!("Failed to parse Cloudflare record ID")))?
        .to_string();

    Ok((zone_id, record_id))
}

pub async fn delete_dns_record(
    zone_id: &str, 
    record_id: &str,
    custom_token: Option<&str>,
) -> Result<(), AppError> {
    let token = match custom_token {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => std::env::var("CLOUDFLARE_API_TOKEN").map_err(|_| AppError::Fatal(anyhow::anyhow!("Missing CLOUDFLARE_API_TOKEN")))?,
    };
    let url = format!("https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}", zone_id, record_id);
    
    let client = reqwest::Client::new();
    let response = client
        .delete(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| AppError::Infrastructure(e.to_string()))?;

    if !response.status().is_success() {
        return Err(AppError::Infrastructure("Failed to delete record from Cloudflare".to_string()));
    }

    Ok(())
}
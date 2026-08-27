use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::{Instant, sleep};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEVICE_CODE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const REFRESH_EARLY: u64 = 5 * 60;
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub access: String,
    pub refresh: String,
    pub expires: u64,
    pub account_id: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    openai_codex: Option<OAuthCredential>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthResponse {
    device_auth_id: String,
    user_code: String,
    interval: Value,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn from_env() -> Result<Self> {
        if let Some(path) = nonempty_env("HABIBI_AUTH_FILE") {
            return Ok(Self {
                path: PathBuf::from(path),
            });
        }

        let config_home = nonempty_env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| nonempty_env("HOME").map(|home| PathBuf::from(home).join(".config")))
            .context("cannot determine config directory; set HABIBI_AUTH_FILE")?;
        Ok(Self {
            path: config_home.join("habibi/auth.json"),
        })
    }

    pub async fn login_openai(&self, client: &Client) -> Result<()> {
        let device = start_device_auth(client).await?;
        let interval_seconds = parse_interval(&device.interval)?;

        println!("Open this URL in your browser:");
        println!("  {DEVICE_VERIFICATION_URI}");
        println!("Enter this one-time code:");
        println!("  {}", device.user_code);
        println!("Waiting for authorization…");

        let authorization = poll_device_auth(
            client,
            &device.device_auth_id,
            &device.user_code,
            interval_seconds,
        )
        .await?;
        let credential = exchange_authorization_code(
            client,
            &authorization.authorization_code,
            &authorization.code_verifier,
        )
        .await?;
        self.save(&credential)?;

        println!(
            "OpenAI login complete. Credentials saved to {}",
            self.path.display()
        );
        Ok(())
    }

    pub async fn valid_openai_credential(&self, client: &Client) -> Result<OAuthCredential> {
        let credential = self.load()?.openai_codex.with_context(|| {
            format!(
                "not logged in; run `habibi login` (auth file: {})",
                self.path.display()
            )
        })?;

        if credential.expires > unix_time() + REFRESH_EARLY {
            return Ok(credential);
        }

        let refreshed = refresh_access_token(client, &credential.refresh).await?;
        self.save(&refreshed)?;
        Ok(refreshed)
    }

    fn load(&self) -> Result<AuthFile> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => serde_json::from_str(&contents)
                .with_context(|| format!("invalid auth file at {}", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AuthFile::default()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read {}", self.path.display()))
            }
        }
    }

    fn save(&self, credential: &OAuthCredential) -> Result<()> {
        let mut auth = self.load()?;
        auth.openai_codex = Some(credential.clone());
        let contents = serde_json::to_vec_pretty(&auth)?;

        let parent = self
            .path
            .parent()
            .context("auth file has no parent directory")?;
        fs::create_dir_all(parent)?;
        let temporary = self.path.with_extension("json.tmp");
        write_private_file(&temporary, &contents)?;
        fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(unix)]
fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents)?;
    Ok(())
}

async fn start_device_auth(client: &Client) -> Result<DeviceAuthResponse> {
    let response = client
        .post(DEVICE_USER_CODE_URL)
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await
        .context("failed to start OpenAI device authorization")?;
    parse_json_response(response, "OpenAI device authorization").await
}

fn parse_interval(value: &Value) -> Result<u64> {
    let interval = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        .context("OpenAI device authorization returned an invalid polling interval")?;
    Ok(interval.max(1))
}

async fn poll_device_auth(
    client: &Client,
    device_auth_id: &str,
    user_code: &str,
    interval_seconds: u64,
) -> Result<DeviceTokenResponse> {
    let deadline = Instant::now() + DEVICE_CODE_TIMEOUT;
    let mut interval = Duration::from_secs(interval_seconds);

    loop {
        if Instant::now() >= deadline {
            bail!("OpenAI device authorization timed out");
        }
        sleep(interval).await;

        let response = client
            .post(DEVICE_TOKEN_URL)
            .json(&serde_json::json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code
            }))
            .send()
            .await
            .context("failed while polling OpenAI device authorization")?;

        if response.status().is_success() {
            return response
                .json()
                .await
                .context("OpenAI returned an invalid device authorization response");
        }
        if matches!(
            response.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ) {
            continue;
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if body.contains("authorization_pending") {
            continue;
        }
        if body.contains("slow_down") {
            interval += Duration::from_secs(5);
            continue;
        }
        bail!("OpenAI device authorization failed ({status}): {body}");
    }
}

async fn exchange_authorization_code(
    client: &Client,
    code: &str,
    verifier: &str,
) -> Result<OAuthCredential> {
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", DEVICE_REDIRECT_URI),
        ])
        .send()
        .await
        .context("failed to exchange OpenAI authorization code")?;
    credential_from_response(response, "exchange").await
}

async fn refresh_access_token(client: &Client, refresh_token: &str) -> Result<OAuthCredential> {
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .await
        .context("failed to refresh OpenAI access token")?;
    credential_from_response(response, "refresh").await
}

async fn credential_from_response(
    response: reqwest::Response,
    operation: &str,
) -> Result<OAuthCredential> {
    let token: TokenResponse =
        parse_json_response(response, &format!("OpenAI token {operation}")).await?;
    let account_id = extract_account_id(&token.access_token)?;
    Ok(OAuthCredential {
        access: token.access_token,
        refresh: token.refresh_token,
        expires: unix_time() + token.expires_in,
        account_id,
    })
}

async fn parse_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    operation: &str,
) -> Result<T> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!("{operation} failed ({status}): {body}");
    }
    serde_json::from_str(&body).with_context(|| format!("{operation} returned invalid JSON"))
}

fn extract_account_id(access_token: &str) -> Result<String> {
    let payload = access_token
        .split('.')
        .nth(1)
        .context("OpenAI access token is not a JWT")?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .context("OpenAI access token has an invalid JWT payload")?;
    let claims: Value = serde_json::from_slice(&decoded)?;
    claims
        .get(JWT_CLAIM_PATH)
        .and_then(|claim| claim.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("OpenAI access token does not contain a ChatGPT account ID")
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_and_string_intervals() {
        assert_eq!(parse_interval(&Value::from(5)).unwrap(), 5);
        assert_eq!(parse_interval(&Value::from("3")).unwrap(), 3);
        assert!(parse_interval(&Value::Null).is_err());
    }
}

use anyhow::{anyhow, Result};
use nanoserde::{DeJson, SerJson};
use native_dialog::{MessageDialog, MessageType};
use reqwest::{Client, StatusCode};
use std::collections::HashMap;
use std::{thread::sleep, time::Duration};

const APPLICATION_ID: &str = "e8eab6e8-494c-4c9c-a800-2836b8468fda";

// this module is brought to you by:
// https://wiki.vg/Microsoft_Authentication_Scheme
// this is hell.

pub async fn auth(
    refresh_token: Option<String>,
    client: Client,
) -> Result<(String, String, String, String)> {
    let microsoft_auth: DeviceSuccess;

    // if there's a refresh token (most likely).
    if let Some(token) = refresh_token {
        println!("refresh token found.");
        let refresh_response = client
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[
                ("client_id", APPLICATION_ID),
                ("scope", "XboxLive.signin offline_access"),
                ("refresh_token", &token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await;
        match DeJson::deserialize_json(&refresh_response?.text().await?) {
            Ok(response) => {
                microsoft_auth = response;
            }
            Err(_) => microsoft_auth = device_flow(client.clone()).await?,
        }
    } else {
        println!("no refresh token present. :/");
        microsoft_auth = device_flow(client.clone()).await?;
    }

    // most of this is annoying xbox auth.
    let mut body = XboxAuth {
        properties: Properties {
            auth_method: Some("RPS".to_string()),
            site_name: Some("user.auth.xboxlive.com".to_string()),
            rps_ticket: Some(format!("d={}", microsoft_auth.access_token)),
            sandbox_id: None,
            user_tokens: None,
        },
        relying_party: "http://auth.xboxlive.com".to_string(),
        token_type: "JWT".to_string(),
    };

    let xbox_auth: XboxAuthResponse = DeJson::deserialize_json(
        &client
            .post("https://user.auth.xboxlive.com/user/authenticate")
            .body(body.serialize_json())
            .send()
            .await?
            .text()
            .await?,
    )?;

    body = XboxAuth {
        properties: Properties {
            auth_method: None,
            site_name: None,
            rps_ticket: None,
            sandbox_id: Some("RETAIL".to_string()),
            user_tokens: Some(Vec::from([xbox_auth.token])),
        },
        relying_party: "rp://api.minecraftservices.com/".to_string(),
        token_type: "JWT".to_string(),
    };

    let xsts_auth: XboxAuthResponse = DeJson::deserialize_json(
        &client
            .post("https://xsts.auth.xboxlive.com/xsts/authorize")
            .body(body.serialize_json())
            .send()
            .await?
            .text()
            .await?,
    )?;

    // NOWS the cool shit.
    let minecraft_auth: MinecraftResponse = DeJson::deserialize_json(
        &client
            .post("https://api.minecraftservices.com/authentication/login_with_xbox")
            .body(
                MinecraftAuth {
                    identity_token: format!(
                        "XBL3.0 x={};{}",
                        xsts_auth.display_claims["xui"][0]["uhs"], xsts_auth.token
                    ),
                }
                .serialize_json(),
            )
            .send()
            .await?
            .text()
            .await?,
    )?;

    // we also need the username and uuid.
    let profile_data: MinecraftProfile = DeJson::deserialize_json(
        &client
            .get("https://api.minecraftservices.com/minecraft/profile")
            .header(
                "Authorization",
                &format!("Bearer {}", minecraft_auth.access_token),
            )
            .send()
            .await?
            .text()
            .await?,
    )?;

    println!("logged in as {}.", profile_data.name);

    Ok((
        profile_data.name,
        profile_data.id,
        minecraft_auth.access_token,
        microsoft_auth.refresh_token,
    ))
}

async fn device_flow(client: Client) -> Result<DeviceSuccess> {
    println!("starting device flow procedure.");

    // gotta ask for a code to login externally.
    let response: DeviceResponse = DeJson::deserialize_json(
        &client
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
            .form(&[
                ("client_id", APPLICATION_ID),
                ("scope", "XboxLive.signin offline_access"),
            ])
            .send()
            .await?
            .text()
            .await?,
    )?;

    let mut successful_auth: Option<DeviceSuccess> = None;

    // docs say this crashes on macos IDK
    MessageDialog::new()
        .set_type(MessageType::Info)
        .set_title("please auth to microsoft... :/")
        .set_text(&format!(
            "login to: {}\nusing code: {}",
            response.verification_uri, response.user_code
        ))
        .show_alert()?;
    while successful_auth.is_none() {
        // now we gotta poll this until it's authed.
        sleep(Duration::from_secs(response.interval));
        println!("polling...");
        let response = client
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", APPLICATION_ID),
                ("device_code", &response.device_code),
            ])
            .send()
            .await?;
        // if the user is successfully authenticated.
        match response.status() {
            StatusCode::OK => {
                println!("authenticated to microsoft!");
                successful_auth = DeJson::deserialize_json(&response.text().await?)?;
            }
            StatusCode::FORBIDDEN => {
                let response: DeviceError = DeJson::deserialize_json(&response.text().await?)?;
                match response.error {
                    AuthError::AuthorizationPending => {}
                    _ => return Err(anyhow!("error authenticating to microsoft.")),
                }
            }
            _ => return Err(anyhow!("error authenticating to microsoft.")),
        }
    }

    // this unwraps because it must be Some().
    Ok(successful_auth.unwrap())
}

// microsoft stuff.

#[derive(DeJson)]
struct DeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
}
#[derive(DeJson)]
struct DeviceError {
    error: AuthError,
}
#[derive(DeJson)]
enum AuthError {
    #[nserde(rename = "authorization_pending")]
    AuthorizationPending,
    #[nserde(rename = "authorization_declined")]
    AuthorizationDeclined,
    #[nserde(rename = "bad_verification_code")]
    BadVerificationCode,
    #[nserde(rename = "expired_token")]
    ExpiredToken,
}
#[derive(DeJson)]
struct DeviceSuccess {
    access_token: String,
    refresh_token: String,
}

// xbox stuff.

#[derive(SerJson)]
struct XboxAuth {
    #[nserde(rename = "Properties")]
    properties: Properties,
    #[nserde(rename = "RelyingParty")]
    relying_party: String,
    #[nserde(rename = "TokenType")]
    token_type: String,
}
#[derive(SerJson)]
struct Properties {
    #[nserde(rename = "AuthMethod")]
    auth_method: Option<String>,
    #[nserde(rename = "SiteName")]
    site_name: Option<String>,
    #[nserde(rename = "RpsTicket")]
    rps_ticket: Option<String>,
    #[nserde(rename = "SandboxId")]
    sandbox_id: Option<String>,
    #[nserde(rename = "UserTokens")]
    user_tokens: Option<Vec<String>>,
}
#[derive(DeJson)]
struct XboxAuthResponse {
    #[nserde(rename = "Token")]
    token: String,
    #[nserde(rename = "DisplayClaims")]
    display_claims: HashMap<String, Vec<HashMap<String, String>>>,
}

// minecraft stuff.

#[derive(SerJson)]
struct MinecraftAuth {
    #[nserde(rename = "identityToken")]
    identity_token: String,
}

#[derive(DeJson)]
struct MinecraftResponse {
    access_token: String,
}

#[derive(DeJson)]
struct MinecraftProfile {
    id: String,
    name: String,
}

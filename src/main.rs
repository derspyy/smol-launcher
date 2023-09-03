mod auth;
mod setup;

use anyhow::Result;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::thread;

use nanoserde::{DeJson, SerJson};

const MOJANG_META: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

fn main() -> Result<()> {
    // setup http client.
    let client = ureq::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .build();

    let version_manifest: VersionManifest =
        DeJson::deserialize_json(&client.get(MOJANG_META).call()?.into_string()?)?;

    let snapshot = false;
    // this version is the one the user wants to play.
    let version_string: String;
    // this should only run if there was no previously defined version (currently always).
    {
        version_string = match snapshot {
            false => version_manifest.latest.release,
            true => version_manifest.latest.snapshot,
        }
    }

    // this should ALWAYS be Some().
    let mut minecraft_version: Option<Version> = None;
    for version in version_manifest.versions {
        if version.id == version_string {
            minecraft_version = Some(version)
        }
    }
    let version = minecraft_version.unwrap();

    // this panics because we need a directory.
    let project_dir = directories::ProjectDirs::from("", "piuvas", "smol launcher").unwrap();
    let data = fs::read_to_string(project_dir.data_dir().join("data.json")).unwrap_or_default();
    let mut app_data: AppData = DeJson::deserialize_json(&data).unwrap_or_default();

    let mut setup_handle = None;

    // setup shouldn't run if it's ran before. >;]
    if !app_data.versions.contains(&version_string) {
        let client = client.clone();
        let project_dir = project_dir.clone();
        setup_handle = Some(thread::spawn(move || {
            setup::setup(version, project_dir.cache_dir().to_path_buf(), client)
        }));
        app_data.versions.push(version_string);
    }

    // authenticates for the launcher data.
    let (username, uuid, access_token, refresh_token) = auth::auth(app_data.refresh_token, client)?;
    app_data.refresh_token = Some(refresh_token);

    if let Some(handle) = setup_handle {
        handle.join().unwrap()?;
    }

    //let's write that data back.
    let mut data_file = fs::File::open(project_dir.data_dir().join("data.json"))?;
    write!(data_file, "{}", app_data.serialize_json())?;

    // starts the game!
    Command::new("java").arg("-version").spawn().expect("woops");

    Ok(())
}

#[derive(DeJson)]
struct VersionManifest {
    latest: Latest,
    versions: Vec<Version>,
}
#[derive(DeJson)]
struct Latest {
    release: String,
    snapshot: String,
}
#[derive(DeJson)]
pub struct Version {
    pub id: String,
    pub url: String,
}

// data file :)
#[derive(DeJson, SerJson, Default)]
struct AppData {
    versions: Vec<String>,
    refresh_token: Option<String>,
}

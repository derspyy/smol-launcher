#![windows_subsystem = "windows"]

mod auth;
mod setup;

use anyhow::Result;
use std::process::Command;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use nanoserde::{DeJson, SerJson};

const MOJANG_META: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[tokio::main]
async fn main() -> Result<()> {
    // setup http client.
    println!("setting up reqwest.");
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;

    let version_manifest: VersionManifest =
        DeJson::deserialize_json(&client.get(MOJANG_META).send().await?.text().await?)?;

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
    let version = minecraft_version.expect("no minecraft version??");
    println!("version: {}.", version_string);
    // this panics because we need a directory.
    let project_dir = directories::ProjectDirs::from("", "piuvas", "smol launcher").unwrap();
    let data = fs::read_to_string(project_dir.data_dir().join("data.json"))
        .await
        .unwrap_or_default();
    let mut app_data: AppData = DeJson::deserialize_json(&data).unwrap_or_default();

    let data_dir = project_dir.data_dir();
    let data_dir_string = data_dir.display();

    let mut setup_handle = None;

    // setup shouldn't run if it's ran before. >;]
    if !app_data.versions.contains(&version_string) {
        println!("installing version...");
        let client = client.clone();
        let project_dir = project_dir.clone();
        setup_handle = Some(tokio::spawn(setup::setup(
            version,
            project_dir.data_dir().to_path_buf(),
            client,
        )));
        app_data.versions.push(version_string.clone());
    } else {
        println!("version is already installed.");
    }

    // authenticates for the launcher data.
    let (username, uuid, access_token) = auth::auth(client, app_data.user_uuid).await?;
    app_data.user_uuid = Some(uuid.clone());

    if let Some(handle) = setup_handle {
        app_data.classpath = Some(handle.await.unwrap()?);
    }

    let classpath = app_data
        .classpath
        .as_ref()
        .expect("we should already have a classpath by now!!!");

    //let's write that data back.
    fs::create_dir_all(project_dir.data_dir()).await?;
    let data_file_path = project_dir.data_dir().join("data.json");
    let mut data_file = fs::File::create(data_file_path).await?;
    data_file
        .write_all(app_data.serialize_json().as_bytes())
        .await?;

    // makes sure there's a minecraft folder.
    let dot_minecraft = data_dir.join(".minecraft");
    if !dot_minecraft.exists() {
        fs::create_dir(dot_minecraft).await?;
    }

    // starts the game!
    println!("starting...");
    let mut binding = Command::new("java");
    let mut command = binding.args([
        &format!("-Djava.library.path={}/libraries", data_dir_string),
        "-Dminecraft.launcher.brand=smol.",
        concat!("-Dminecraft.launcher.version=", env!("CARGO_PKG_VERSION")),
        "-cp",
        classpath,
        "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump",
    ]);

    if cfg!(target_os = "macos") {
        command = command.arg("-XstartOnFirstThread");
    } else if cfg!(target_os = "windows") {
        command = command.args(["-Dos.name=Windows 10", "-Dos.version=10.0"])
    }

    command = command.arg("net.minecraft.client.main.Main");

    // game args...
    command = command.args([
        "--username",
        &username,
        "--version",
        &version_string,
        "--gameDir",
        &format!("{}/.minecraft", data_dir_string),
        "--assetsDir",
        &format!("{}/assets", data_dir_string),
        "--assetIndex",
        &version_string,
        "--uuid",
        &uuid,
        "--accessToken",
        &access_token,
        "--userType",
        "msa",
        "--versionType",
        match snapshot {
            true => "snapshot",
            false => "release",
        },
    ]);

    command.current_dir(format!("{}/.minecraft", data_dir_string));
    command.spawn()?;

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
    classpath: Option<String>,
    user_uuid: Option<String>,
}

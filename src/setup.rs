use super::Version;

use anyhow::{anyhow, Result};
use futures::future::try_join_all;
use nanoserde::DeJson;
use reqwest::Client as HttpClient;
use sha1_smol::Sha1;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs::{create_dir_all, File};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

static PERMITS: Semaphore = Semaphore::const_new(100);

pub async fn setup(version: Version, data_dir: PathBuf, client: HttpClient) -> Result<String> {
    let version_data: VersionData =
        DeJson::deserialize_json(&client.get(&version.url).send().await?.text().await?)?;
    #[cfg(target_os = "linux")]
    let os = Os::Linux;
    #[cfg(target_os = "windows")]
    let os = Os::Windows;
    #[cfg(target_os = "macos")]
    let os = Os::Osx;

    let mut futures = Vec::new();

    // gotta return the classpaths...
    let mut classpath = String::new();
    let separator = match cfg!(target_os = "windows") {
        true => ';',
        false => ':',
    };

    // downloads the libraries.
    for library in version_data.libraries {
        let path = data_dir
            .join("libraries")
            .join(library.downloads.artifact.path);
        // either there's no os rule or it's the same as current os.
        if library.rules.is_none() || library.rules.is_some_and(|x| x[0].os.name == os) {
            if !path.exists() {
                let client = client.clone();
                futures.push(tokio::spawn(download(
                    library.downloads.artifact.url,
                    library.downloads.artifact.sha1,
                    path,
                    client,
                )));
            } else {
                let path_string = path.display();
                classpath.push_str(&path_string.to_string());
                classpath.push(separator);
            }
        }
    }

    // downloads the client.
    // you need this format!() because set_extension() doesn't care about dots in the filename.
    let path = data_dir
        .join("versions")
        .join(format!("{}.jar", version.id));
    if !path.exists() {
        futures.push(tokio::spawn(download(
            version_data.downloads.client.url,
            version_data.downloads.client.sha1,
            path,
            client.clone(),
        )));
    } else {
        let path_string = path.display();
        classpath.push_str(&path_string.to_string());
        classpath.push(separator);
    }

    let results = try_join_all(futures).await?;

    for result in results {
        let path = result?;
        classpath.push_str(path.to_str().ok_or(anyhow!("unsupported path!"))?);
        classpath.push(separator);
    }

    classpath.pop();

    let mut inputs = Vec::new();

    // downloads the assets.
    let asset_index_string = client
        .get(&version_data.asset_index.url)
        .send()
        .await?
        .text()
        .await?;
    let asset_index: Assets = DeJson::deserialize_json(&asset_index_string)?;

    // we need an asset index file.
    let path = data_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version.id));
    if !path.exists() {
        create_dir_all(data_dir.join("assets").join("indexes")).await?;
        let mut data_file = File::create(path).await?;
        data_file.write_all(asset_index_string.as_bytes()).await?;
    }

    for (_, data) in asset_index.objects {
        let two = &data.hash[0..2];
        let path = data_dir
            .join("assets")
            .join("objects")
            .join(two)
            .join(&data.hash);
        if !path.exists() {
            let url = format!(
                "https://resources.download.minecraft.net/{}/{}",
                two, data.hash
            );
            let client = client.clone();
            inputs.push((url, data.hash, path, client));
        }
    }

    // batch downloads.
    let mut handles = Vec::new();
    for (url, hash, path, client) in inputs {
        let handle = tokio::spawn(download(url, hash, path, client));
        handles.push(handle);
    }
    try_join_all(handles).await?;

    println!("finished setup.");

    Ok(classpath)
}

async fn download(
    url: String,
    hash: String,
    mut path: PathBuf,
    client: HttpClient,
) -> Result<PathBuf> {
    let mut hasher = Sha1::new();
    println!("downloading file: {} -> {}", url, path.display());
    let full_path = path.clone();
    path.pop();
    let _permit = PERMITS.acquire().await?;
    create_dir_all(path).await?;
    let bytes = client.get(&url).send().await?.bytes().await?;
    drop(client);
    hasher.update(&bytes);
    assert_eq!(
        hasher.digest().to_string(),
        hash,
        "{} failed integrity check.",
        url
    );
    let mut new_file = File::create(&full_path).await?;
    new_file.write_all(&bytes).await?;
    Ok(full_path)
}

#[derive(DeJson)]
struct VersionData {
    libraries: Vec<Library>,
    downloads: Downloads,
    #[nserde(rename = "assetIndex")]
    asset_index: AssetIndex,
}

#[derive(DeJson)]
struct Library {
    downloads: LibDownloads,
    rules: Option<Vec<Rules>>,
}

#[derive(DeJson)]
struct LibDownloads {
    artifact: Artifact,
}

#[derive(DeJson)]
struct Artifact {
    path: String,
    sha1: String,
    url: String,
}

#[derive(DeJson)]
struct Rules {
    os: OsStruct,
}

#[derive(DeJson)]
struct OsStruct {
    name: Os,
}

#[derive(DeJson, PartialEq)]
enum Os {
    #[nserde(rename = "osx")]
    Osx,
    #[nserde(rename = "windows")]
    Windows,
    #[nserde(rename = "linux")]
    Linux,
}

#[derive(DeJson)]
struct Downloads {
    client: Client,
}

#[derive(DeJson)]
struct Client {
    sha1: String,
    url: String,
}

#[derive(DeJson)]
struct AssetIndex {
    url: String,
}

#[derive(DeJson)]
struct Assets {
    objects: HashMap<String, Asset>,
}

#[derive(DeJson)]
struct Asset {
    hash: String,
}

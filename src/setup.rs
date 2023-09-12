use super::Version;

use anyhow::Result;
use nanoserde::DeJson;
use sha1_smol::Sha1;
use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use ureq::Agent;

pub fn setup(version: Version, data_dir: PathBuf, client: Agent) -> Result<String> {
    let version_data: VersionData =
        DeJson::deserialize_json(&client.get(&version.url).call()?.into_string()?)?;
    #[cfg(target_os = "linux")]
    let os = Os::Linux;
    #[cfg(target_os = "windows")]
    let os = Os::Windows;
    #[cfg(target_os = "macos")]
    let os = Os::Osx;

    let mut handles = Vec::new();

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
        if !path.exists()
            && (library.rules.is_none() || library.rules.is_some_and(|x| x[0].os.name == os))
        {
            let client = client.clone();
            handles.push(thread::spawn(|| {
                download(
                    library.downloads.artifact.url,
                    library.downloads.artifact.sha1,
                    path,
                    client,
                )
            }));
        } else {
            let path_string = path.display();
            classpath.push_str(&path_string.to_string());
            classpath.push(separator);
        }
    }

    for handle in handles {
        handle.join().unwrap()?;
    }

    let mut handles = Vec::new();

    // downloads the assets.
    let asset_index_string = client
        .get(&version_data.asset_index.url)
        .call()?
        .into_string()?;
    let asset_index: Assets = DeJson::deserialize_json(&asset_index_string)?;

    // we need an asset index file.
    let path = data_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version.id));
    if !path.exists() {
        create_dir_all(data_dir.join("assets").join("indexes"))?;
        let mut data_file = std::fs::File::create(path)?;
        write!(data_file, "{}", asset_index_string)?;
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
            download(url, data.hash, path, client)?;
        }
    }

    // downloads the client.
    // you need this format!() because set_extension() doesn't care about dots in the filename.
    let path = data_dir
        .join("versions")
        .join(format!("{}.jar", version.id));
    if !path.exists() {
        handles.push(thread::spawn(|| {
            download(
                version_data.downloads.client.url,
                version_data.downloads.client.sha1,
                path,
                client,
            )
        }));
    } else {
        let path_string = path.display();
        classpath.push_str(&path_string.to_string());
        classpath.push(separator);
    }

    for handle in handles {
        classpath.push_str(&handle.join().unwrap()?.to_string_lossy());
        classpath.push(separator);
    }

    classpath.pop();

    println!("finished setup.");

    Ok(classpath)
}

fn download(url: String, hash: String, mut path: PathBuf, client: Agent) -> Result<PathBuf> {
    let mut hasher = Sha1::new();
    println!("downloading file: {} -> {}", url, path.display());
    let full_path = path.clone();
    path.pop();
    create_dir_all(path)?;
    let mut new_file = File::create(&full_path)?;
    let mut bytes: Vec<u8> = Vec::new();
    client
        .get(&url)
        .call()?
        .into_reader()
        .read_to_end(&mut bytes)?;
    hasher.update(&bytes);
    assert_eq!(hasher.digest().to_string(), hash);
    new_file.write_all(&bytes)?;
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

use super::Version;

use anyhow::Result;
use nanoserde::DeJson;
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use ureq::Agent;

pub fn setup(version: Version, cache_dir: PathBuf, client: Agent) -> Result<()> {
    let version_data: VersionData =
        DeJson::deserialize_json(&client.get(&version.url).call()?.into_string()?)?;

    let mut handles = Vec::new();
    for library in version_data.libraries {
        let path = cache_dir.join(library.downloads.artifact.path);
        if !path.exists() {
            let client = client.clone();
            handles.push(thread::spawn(|| {
                download_library(library.downloads.artifact.url, path, client)
            }));
        }
    }

    for handle in handles {
        handle.join().unwrap()?;
    }

    Ok(())
}

fn download_library(url: String, mut path: PathBuf, client: Agent) -> Result<()> {
    let full_path = path.clone();
    path.pop();
    create_dir_all(path)?;
    let mut new_file = File::create(full_path)?;
    let mut bytes: Vec<u8> = Vec::new();
    client
        .get(&url)
        .call()?
        .into_reader()
        .read_to_end(&mut bytes)?;
    new_file.write_all(&bytes)?;
    Ok(())
}

#[derive(DeJson)]
struct VersionData {
    libraries: Vec<Library>,
}

#[derive(DeJson)]
struct Library {
    downloads: Downloads,
}

#[derive(DeJson)]
struct Downloads {
    artifact: Artifact,
}

#[derive(DeJson)]
struct Artifact {
    path: String,
    sha1: String,
    url: String,
}

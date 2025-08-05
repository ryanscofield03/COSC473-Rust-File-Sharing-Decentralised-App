use std::io::Error;
use libp2p::PeerId;
use tokio::fs::{create_dir_all, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FILES_DIR: &str = "/files";

/// helper function for generating path to files directory for this peer
/// (required since we have many peers in one directory)
fn generate_path_to_files_directory(peer_id: PeerId) -> String {
    "./".to_string() + peer_id.to_string().as_str() + FILES_DIR
}

/// helper function for generating path to specific file
/// (required since we have many peers in one directory)
fn generate_path_to_filename(peer_id: PeerId, filename: String) -> String {
    generate_path_to_files_directory(peer_id) + "/".to_string().as_str() + filename.as_str()
}

/// Create directory(s) for the app file storage
pub(crate) async fn create_directories(peer_id: PeerId) {
    create_dir_all(generate_path_to_files_directory(peer_id)).await.unwrap();
}

/// try and retrieve a file as a vec of u8s from the app's file directory
pub(crate) async fn retrieve_data_from_file(
    peer_id: PeerId,
    filename: String
) -> Result<Vec<u8>, Error> {
    let file_path = generate_path_to_filename(peer_id, filename);
    match File::open(file_path).await {
        Ok(mut file) => {
            let mut buffer = Vec::new();
            let _ = file.read_to_end(&mut buffer).await;
            Ok(buffer)
        }
        Err(err) => Err(err)
    }
}

/// create and write data to a new file in the app's file directory
pub(crate) async fn write_data_to_file(
    peer_id: PeerId,
    filename: String,
    data: Vec<u8>
) -> Result<(), Error> {
    let file_path = generate_path_to_filename(peer_id, filename);
    match File::create(file_path).await {
        Ok(mut file) => {
            file.write_all(&data).await
        }
        Err(err) => {
            Err(err)
        }
    }
}
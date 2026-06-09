use std::process::Command;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use crate::utils::error::AppError;

/// Generates a new SSH key pair (Ed25519, falling back to RSA if needed).
/// Returns a tuple of (private_key, public_key).
pub fn generate_ssh_keypair() -> Result<(String, String), AppError> {
    // Create a temporary directory in the current working directory to keep it in the workspace
    let run_id = Uuid::new_v4();
    let temp_dir_path = PathBuf::from(format!("./.tmp_ssh_{}", run_id));
    
    if let Err(e) = fs::create_dir_all(&temp_dir_path) {
        return Err(AppError::Fatal(anyhow::anyhow!("Failed to create temporary directory for SSH keygen: {}", e)));
    }

    let key_file = temp_dir_path.join("id_git");
    let key_file_str = key_file.to_string_lossy().to_string();

    // Try Ed25519 first (modern and fast)
    let mut cmd = Command::new("ssh-keygen");
    cmd.args(&["-t", "ed25519", "-N", "", "-f", &key_file_str]);

    let output = cmd.output();
    let mut success = false;

    if let Ok(out) = output {
        if out.status.success() {
            success = true;
        }
    }

    // Fallback to RSA if Ed25519 fails
    if !success {
        let mut fallback_cmd = Command::new("ssh-keygen");
        fallback_cmd.args(&["-t", "rsa", "-b", "2048", "-N", "", "-f", &key_file_str]);
        match fallback_cmd.output() {
            Ok(out) if out.status.success() => {
                success = true;
            }
            Ok(out) => {
                let err_msg = String::from_utf8_lossy(&out.stderr).to_string();
                let _ = fs::remove_dir_all(&temp_dir_path);
                return Err(AppError::Fatal(anyhow::anyhow!("ssh-keygen failed: {}", err_msg)));
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&temp_dir_path);
                return Err(AppError::Fatal(anyhow::anyhow!("Failed to execute ssh-keygen: {}", e)));
            }
        }
    }

    // Read the keys
    let pub_key_file = temp_dir_path.join("id_git.pub");
    
    let private_key = match fs::read_to_string(&key_file) {
        Ok(content) => content,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_dir_path);
            return Err(AppError::Fatal(anyhow::anyhow!("Failed to read generated private key: {}", e)));
        }
    };

    let public_key = match fs::read_to_string(&pub_key_file) {
        Ok(content) => content,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_dir_path);
            return Err(AppError::Fatal(anyhow::anyhow!("Failed to read generated public key: {}", e)));
        }
    };

    // Clean up temporary files
    let _ = fs::remove_dir_all(&temp_dir_path);

    Ok((private_key.trim().to_string(), public_key.trim().to_string()))
}

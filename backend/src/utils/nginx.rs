use std::fs;
use std::path::Path;
use crate::utils::nginx_templates::NginxTemplates;
use crate::utils::error::AppError;

#[cfg(unix)]
use std::os::unix::fs::symlink;

const SITES_AVAILABLE: &str = "/etc/nginx/sites-available";
const SITES_ENABLED: &str = "/etc/nginx/sites-enabled";
const SSL_DIR: &str = "/etc/ssl/hermes";

pub struct NginxManager;

impl NginxManager {
    pub fn deploy_site(
        domain_type: &str, 
        domain: &str, 
        nginx_target_host: Option<&str>, 
        nginx_root_path: Option<&str>,   
        client_max_body_size: i32,
        is_ssl: bool,
        cert_path: &str,
        key_path: &str,
        nginx_config_content: Option<&str>,
    ) -> Result<String, AppError> {
        
        let config_content = NginxTemplates::generate_config(
            domain_type, 
            domain, 
            nginx_target_host, 
            nginx_root_path, 
            client_max_body_size, 
            is_ssl, 
            cert_path, 
            key_path, 
            nginx_config_content
        ).map_err(|e| AppError::Validation(format!("Template generation failed: {}", e)))?;

        #[cfg(unix)]
        {
            if is_ssl {
                fs::create_dir_all(SSL_DIR)
                    .map_err(|e| AppError::Infrastructure(format!("Failed to create SSL directory: {}", e)))?;

                if !Path::new(cert_path).exists() || !Path::new(key_path).exists() {
                    let ssl_output = Command::new("sudo")
                        .args(&[
                            "openssl", "req", "-x509", "-nodes", "-days", "365",
                            "-newkey", "rsa:2048",
                            "-keyout", key_path,
                            "-out", cert_path,
                            "-subj", &format!("/CN={}", domain)
                        ])
                        .output()
                        .map_err(|e| AppError::Infrastructure(format!("Failed to execute OpenSSL command: {}", e)))?;

                    if !ssl_output.status.success() {
                        let ssl_err = String::from_utf8_lossy(&ssl_output.stderr);
                        return Err(AppError::Infrastructure(format!("OpenSSL generation failed: {}", ssl_err)));
                    }
                }
            }

            let available_path = format!("{}/{}", SITES_AVAILABLE, domain);
            let enabled_path = format!("{}/{}", SITES_ENABLED, domain);

            fs::write(&available_path, &config_content)
                .map_err(|e| AppError::Infrastructure(format!("Failed to write config file: {}", e)))?;

            if Path::new(&enabled_path).exists() {
                let _ = fs::remove_file(&enabled_path);
            }
            
            symlink(&available_path, &enabled_path)
                .map_err(|e| AppError::Infrastructure(format!("Failed to create symlink: {}", e)))?;

            let test_output = Command::new("sudo")
                .arg("nginx")
                .arg("-t")
                .output()
                .map_err(|e| AppError::Infrastructure(format!("Failed to execute nginx validation: {}", e)))?;

            if !test_output.status.success() {
                let _ = fs::remove_file(&enabled_path); 
                let error_msg = String::from_utf8_lossy(&test_output.stderr);
                return Err(AppError::Infrastructure(format!("Nginx validation failed: {}", error_msg)));
            }

            let reload_output = Command::new("sudo") 
                .arg("systemctl")
                .arg("reload")
                .arg("nginx")
                .output()
                .map_err(|e| AppError::Infrastructure(format!("Failed to reload nginx service: {}", e)))?;

            if !reload_output.status.success() {
                return Err(AppError::Infrastructure("Failed to execute systemctl reload nginx".to_string()));
            }
        }
        
        #[cfg(not(unix))]
        {
            let _ = fs::create_dir_all("temp_nginx");
            let local_path = format!("temp_nginx/{}", domain);
            
            fs::write(&local_path, &config_content)
                .map_err(|e| AppError::Infrastructure(format!("Failed to write simulation file: {}", e)))?;

            if is_ssl {
                let _ = fs::create_dir_all("temp_nginx/ssl");
                let _ = fs::write(cert_path.replace("/etc/ssl/hermes", "temp_nginx/ssl"), "mock cert data");
                let _ = fs::write(key_path.replace("/etc/ssl/hermes", "temp_nginx/ssl"), "mock key data");
            }
        }

        Ok(config_content)
    }

    pub fn delete_site(domain: &str) -> Result<(), AppError> {
        #[cfg(unix)]
        {
            let available_path = format!("{}/{}", SITES_AVAILABLE, domain);
            let enabled_path = format!("{}/{}", SITES_ENABLED, domain);

            if Path::new(&enabled_path).exists() {
                let _ = fs::remove_file(&enabled_path);
            }
            if Path::new(&available_path).exists() {
                let _ = fs::remove_file(&available_path);
            }

            let cert_path = format!("{}/{}.crt", SSL_DIR, domain);
            let key_path = format!("{}/{}.key", SSL_DIR, domain);
            if Path::new(&cert_path).exists() { let _ = fs::remove_file(cert_path); }
            if Path::new(&key_path).exists() { let _ = fs::remove_file(key_path); }

            let _ = Command::new("sudo").arg("systemctl").arg("reload").arg("nginx").output();
        }
        
        #[cfg(not(unix))]
        {
            let local_path = format!("temp_nginx/{}", domain);
            if Path::new(&local_path).exists() {
                let _ = fs::remove_file(local_path);
            }
        }

        Ok(())
    }
}
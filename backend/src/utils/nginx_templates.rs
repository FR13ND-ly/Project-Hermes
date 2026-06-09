pub struct NginxTemplates;

impl NginxTemplates {
    const SECURITY_HEADERS: &'static str = r#"
    # Security Headers
    add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;
    add_header X-Frame-Options "SAMEORIGIN" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;
    add_header X-Permitted-Cross-Domain-Policies "none" always;
    add_header Permissions-Policy "camera=(), microphone=(), geolocation=(), payment=()" always;
    "#;

    const OPTIMIZATION_PARAMS: &'static str = r#"
    # Gzip Compression
    gzip on;
    gzip_vary on;
    gzip_proxied any;
    gzip_comp_level 6;
    gzip_types text/plain text/css text/xml application/json application/javascript application/rss+xml application/atom+xml image/svg+xml;

    # Connection Optimizations
    keepalive_timeout 65;
    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;

    # Client Buffers
    client_body_buffer_size 128k;
    client_header_buffer_size 1k;
    large_client_header_buffers 4 8k;
    "#;

    const PROXY_PARAMS: &'static str = r#"
    # Proxy Headers
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    
    # Proxy Timeouts
    proxy_connect_timeout 60s;
    proxy_send_timeout 300s;
    proxy_read_timeout 300s;

    # Proxy Buffers
    proxy_buffer_size 8k;
    proxy_buffers 8 64k;
    proxy_busy_buffers_size 128k;
    "#;

    pub fn generate_config(
        domain_type: &str, 
        domain: &str,
        target_host: Option<&str>,
        root_path: Option<&str>,
        client_max_body_size: i32, 
        is_ssl: bool,
        cert_path: &str,
        key_path: &str,
        custom_config: Option<&str>,
    ) -> Result<String, String> {
        if domain_type == "custom" {
            return custom_config
                .map(|s| s.to_string())
                .ok_or_else(|| "Custom config requires nginx_config_content".to_string());
        }

        let client_max_body = format!("client_max_body_size {}M;", client_max_body_size);

        let content_logic = match domain_type {
            "reverse_proxy" => {
                let raw_host = target_host.ok_or_else(|| "Reverse proxy requires nginx_target_host".to_string())?;
                let clean_host = raw_host.trim_end_matches('/');

                let upstream = if clean_host.starts_with("http://") || clean_host.starts_with("https://") {
                    clean_host.to_string()
                } else {
                    format!("http://{}", clean_host)
                };

                Self::reverse_proxy_template(&upstream)
            }
            "web_server" => {
                let root = root_path.ok_or_else(|| "Web server requires nginx_root_path".to_string())?;
                Self::web_server_template(root)
            }
            "static_host" => {
                let root = root_path.ok_or_else(|| "Static host requires nginx_root_path".to_string())?;
                Self::static_host_template(root)
            }
            _ => return Err(format!("Domain type {} is not supported", domain_type)),
        };

        if is_ssl {
            let http_redirect_block = format!(
                r#"
server {{
    listen 80;
    listen [::]:80;
    server_name {domain} www.{domain};
    
    location /.well-known/acme-challenge/ {{
        root /var/www/certbot;
    }}

    location / {{
        return 301 https://{domain}$request_uri;
    }}
}}
"#, 
                domain = domain
            );

            let https_block = format!(
                r#"
server {{
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name {domain} www.{domain};
    
    {client_max_body}
    
    # SSL Configuration
    ssl_certificate {cert};
    ssl_certificate_key {key};
    ssl_session_timeout 1d;
    ssl_session_cache shared:SSL:50m;
    ssl_session_tickets off;

    # Modern Cipher Suites
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers 'ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305:DHE-RSA-AES128-GCM-SHA256:DHE-RSA-AES256-GCM-SHA384';
    ssl_prefer_server_ciphers off;

    # OCSP Stapling
    ssl_stapling on;
    ssl_stapling_verify on;
    
    resolver 127.0.0.11 1.1.1.1 8.8.8.8 valid=300s;
    resolver_timeout 2s;

    access_log /var/log/nginx/{domain}.access.log;
    error_log /var/log/nginx/{domain}.error.log warn;
    
    error_page 500 502 503 504 /50x.html;
    location = /50x.html {{
        root /usr/share/nginx/html;
    }}

    {security_headers}
    {optimization_params}

    {content}
}}
"#,
                domain = domain,
                client_max_body = client_max_body,
                cert = cert_path,
                key = key_path,
                security_headers = Self::SECURITY_HEADERS,
                optimization_params = Self::OPTIMIZATION_PARAMS,
                content = content_logic
            );

            Ok(format!("{}\n{}", http_redirect_block, https_block))
        } else {
            let http_block = format!(
                r#"
server {{
    listen 80;
    listen [::]:80;
    server_name {domain} www.{domain};
    
    {client_max_body}

    access_log /var/log/nginx/{domain}.access.log;
    error_log /var/log/nginx/{domain}.error.log warn;

    {optimization_params}

    {content}
}}
"#, 
                domain = domain,
                client_max_body = client_max_body,
                optimization_params = Self::OPTIMIZATION_PARAMS,
                content = content_logic
            );

            Ok(http_block)
        }
    }

    fn reverse_proxy_template(upstream: &str) -> String {
        format!(
            r#"
    location / {{
        proxy_pass {upstream};
        proxy_set_header Host $http_host;
        {proxy_params}
        proxy_cache_bypass $http_upgrade;
    }}
"#,
            upstream = upstream,
            proxy_params = Self::PROXY_PARAMS
        )
    }

    fn web_server_template(root_path: &str) -> String {
        format!(
            r#"
    root {root};
    index index.html index.htm;

    location / {{
        try_files $uri $uri/ /index.html;
    }}

    location ~ /\. {{
        deny all;
        access_log off;
        log_not_found off;
    }}
"#,
            root = root_path
        )
    }

    fn static_host_template(root_path: &str) -> String {
        format!(
            r#"
    location / {{
        alias {root}/;
        try_files $uri $uri/ =404;
        expires 30d;
        add_header Cache-Control "public, no-transform";
        access_log off;
    }}
"#,
            root = root_path
        )
    }
}
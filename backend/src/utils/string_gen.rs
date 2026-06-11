pub fn generate_secure_string(length: usize) -> String {
    nanoid::nanoid!(length)
}

pub fn sanitize_k8s_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
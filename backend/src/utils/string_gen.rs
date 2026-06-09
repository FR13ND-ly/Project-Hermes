pub fn generate_secure_string(length: usize) -> String {
    nanoid::nanoid!(length)
}
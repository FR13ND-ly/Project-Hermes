use sqlx::{postgres::PgPoolOptions, Row};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;

    // First list columns
    let cols = sqlx::query(
        "SELECT column_name FROM information_schema.columns WHERE table_name = 'database_backups'"
    )
    .fetch_all(&pool)
    .await?;

    println!("Columns in database_backups:");
    for r in &cols {
        let name: String = r.get(0);
        println!("  - {}", name);
    }

    // Dynamic select
    let rows = sqlx::query("SELECT * FROM database_backups")
        .fetch_all(&pool)
        .await?;

    println!("\n--- Backups List ---");
    for row in rows {
        let id: uuid::Uuid = row.get("id");
        let db_id: uuid::Uuid = row.get("database_id");
        let filename: String = row.get("filename");
        let status: String = row.get("status");
        println!(
            "ID: {}, DB_ID: {}, File: {}, Status: {}",
            id, db_id, filename, status
        );
    }

    let dbs = sqlx::query("SELECT id, name, type, container_name, is_external FROM databases")
        .fetch_all(&pool)
        .await?;

    println!("\n--- Databases List ---");
    for d in dbs {
        let id: uuid::Uuid = d.get("id");
        let name: String = d.get("name");
        let db_type: String = d.get("type"); // wait, type might be an enum, let's print it as string
        let container: String = d.get("container_name");
        let is_ext: bool = d.get("is_external");
        println!(
            "ID: {}, Name: {}, Type: {}, Container: {}, External: {}",
            id, name, db_type, container, is_ext
        );
    }

    Ok(())
}

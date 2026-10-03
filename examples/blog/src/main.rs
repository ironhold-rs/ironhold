#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://blog.db".to_owned());
    let db = ironhold::prelude::SqliteDb::connect(&database_url).await?;
    sqlx::migrate!().run(db.pool()).await?;

    blog::app(db).serve().await?;
    Ok(())
}

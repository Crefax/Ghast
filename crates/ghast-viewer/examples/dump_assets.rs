//! Builds the page's block data and texture pack from a client jar and writes them to a folder
//! (to inspect what the server serves).
//!
//! ```text
//! cargo run --example dump_assets -- <client.jar | --download> <out dir>
//! ```

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let jar = args.next().unwrap_or_else(|| "--download".into());
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "assets-out".into()));
    let cache = std::path::PathBuf::from(".cache/ghast-viewer");
    let jar = if jar == "--download" {
        ghast_viewer::assets::jar::client_jar(&cache, None)?
    } else {
        jar.into()
    };
    let t0 = std::time::Instant::now();
    let (data, pack) = ghast_viewer::assets::build(&jar)?;
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("data.json"), &data)?;
    std::fs::write(out.join("pack.bin"), &pack)?;
    println!(
        "built in {} ms: data {} KB, pack {} KB",
        t0.elapsed().as_millis(),
        data.len() >> 10,
        pack.len() >> 10
    );
    Ok(())
}

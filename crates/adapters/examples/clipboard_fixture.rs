//! Test driver: only synthetic content; output never contains clipboard bytes.
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut clipboard = shuttli_adapters::clipboard::open()?;
    if args.is_empty() {
        return Err("usage: clipboard_fixture inspect | text <fixture> | png <path>".into());
    }
    let prior = clipboard.read()?;
    if args[0] == "inspect" {
        println!(
            "generation={} format={:?} bytes={} digest={}",
            prior.stamp.generation,
            prior.payload.as_ref().map(|p| p.meta.format),
            prior.payload.as_ref().map_or(0, |p| p.meta.size),
            hex::encode(prior.stamp.digest)
        );
        return Ok(());
    }
    let (kind, bytes) = match args[0].as_str() {
        "text" => (
            shuttli_model::sync::Format::Text,
            args.get(1).ok_or("missing fixture")?.as_bytes().to_vec(),
        ),
        "png" => (
            shuttli_model::sync::Format::Png,
            std::fs::read(args.get(1).ok_or("missing fixture path")?).map_err(|e| e.to_string())?,
        ),
        _ => return Err("invalid command".into()),
    };
    let p = shuttli_adapters::content::payload(kind, bytes)?;
    let core = shuttli_core::sync::SyncCore::new([0; 32], [0; 16], Default::default());
    let value = clipboard.write(&p, core.authorize_local_copy(prior.stamp, p.meta.clone()))?;
    println!(
        "fixture set; bytes={} digest={}",
        p.meta.size,
        hex::encode(value.stamp.digest)
    );
    std::thread::sleep(std::time::Duration::from_secs(30));
    Ok(())
}

//! Opt-in real Windows clipboard + TLS acceptance against an isolated local agent.
//! Only synthetic fixtures are exchanged. Requires SHUTTLI_DATA_DIR explicitly.
#[cfg(windows)]
fn main() -> Result<(), String> {
    use shuttli_model::sync::{DeliveryState, EventId, Format};
    use shuttli_protocol::{FrameV2 as Frame, Hello};
    use shuttli_transport::{WireVersion, read_live_frame, write_live_frame};
    use std::{process::Command, time::Duration};
    fn cli(args: &[&str]) -> Result<serde_json::Value, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let output = Command::new(exe.parent().unwrap().parent().unwrap().join("shuttli.exe"))
            .args(args)
            .arg("--json")
            .output()
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("CLI failed: {value}"));
        }
        Ok(value)
    }
    let profile =
        std::env::var_os("SHUTTLI_DATA_DIR").ok_or("explicit isolated profile required")?;
    let status = cli(&["status"])?;
    if !status["status"]["settings"]["peers"]
        .as_object()
        .is_some_and(|p| p.is_empty())
    {
        return Err("acceptance requires a fresh profile with no trusted peers".into());
    }
    cli(&["set", "automatic", "off"])?;
    cli(&["set", "notifications", "off"])?;
    let address = shuttli_adapters::discovery::snapshot()?.own;
    let (identity, _) = shuttli_identity::Identity::generate()?;
    let id = identity.id;
    let epoch = shuttli_adapters::random_epoch()?;
    let mut clipboard = shuttli_adapters::clipboard::open()?;
    let saved = clipboard.read()?;
    let core = shuttli_core::sync::SyncCore::new([0; 32], [0; 16], Default::default());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let result = runtime.block_on(async {
        let stream = tokio::net::TcpStream::connect((address,45987)).await.map_err(|e|e.to_string())?;
        let mut tls = tokio_rustls::TlsConnector::from(identity.client)
            .connect(rustls::pki_types::ServerName::try_from("shuttli.local").unwrap(),stream).await.map_err(|e|e.to_string())?;
        let server = shuttli_identity::device_id(tls.get_ref().1.peer_certificates().unwrap()[0].as_ref())?;
        if hex::encode(server) != status["status"]["device"].as_str().unwrap() {return Err("local TLS identity mismatch".into());}
        shuttli_transport::write_hello(&mut tls,&Hello::V1 {name:"Synthetic Windows acceptance peer".into(),epoch}).await?;
        shuttli_transport::read_hello(&mut tls).await?;
        let version = WireVersion::V1;
        if !matches!(read_live_frame(&mut tls,version).await?,Frame::Select {initiator} if initiator==id) {return Err("selection failed".into());}
        write_live_frame(&mut tls,&Frame::Select {initiator:id},version).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let mut png=Vec::new();
        {let mut encoder=png::Encoder::new(&mut png,3,2);encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);
         encoder.write_header().map_err(|e|e.to_string())?.write_image_data(&[10,20,30,0,250,120,30,128,10,220,70,255,1,2,3,255,4,5,6,255,7,8,9,255]).map_err(|e|e.to_string())?;}
        for (sequence,(format,bytes)) in [(Format::Text,"Windows TLS 中文 😀\r\nline\nCR\rend".as_bytes().to_vec()),(Format::Png,png.clone())].into_iter().enumerate() {
            let payload=shuttli_adapters::content::payload(format,bytes)?;
            let event=EventId {origin:id,epoch,seq:sequence as u64+1};
            let outcome=shuttli_transport::send_live_version(&mut tls,shuttli_transport::LiveOffer {event,metadata:&payload.meta,body:&payload.data},||true,|_|Ok(()),||{},version).await?;
            if !matches!(outcome,shuttli_transport::SendOutcome::Applied) {return Err("receiver did not confirm APPLIED".into());}
            if clipboard.read()?.stamp.digest!=payload.meta.digest {return Err("actual Windows clipboard differs from TLS body".into());}
            write_live_frame(&mut tls,&Frame::Status {event},version).await?;
            if !matches!(read_live_frame(&mut tls,version).await?,Frame::Receipt {event:e,state:Some(DeliveryState::Applied)} if e==event) {return Err("durable APPLIED receipt missing".into());}
            println!("PASS: TLS incoming {format:?}, native OS readback and durable APPLIED receipt");
        }
        cli(&["peer",&hex::encode(id),"send","on"])?;
        cli(&["set","send","on"])?;
        for (format,bytes) in [(Format::Text,b"Windows outbound synthetic TLS fixture".to_vec()),(Format::Png,png)] {
        let outgoing=shuttli_adapters::content::payload(format,bytes)?;
        let before=clipboard.read()?;
        clipboard.write(&outgoing,core.authorize_local_copy(before.stamp,outgoing.meta.clone()))?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        cli(&["send"])?;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        loop {
            if tokio::time::Instant::now()>deadline {return Err("outgoing transfer timeout".into());}
            write_live_frame(&mut tls,&Frame::Poll,version).await?;
            match read_live_frame(&mut tls,version).await? {
                Frame::Idle=>tokio::time::sleep(Duration::from_millis(100)).await,
                Frame::Offer {meta,..}=>{
                    write_live_frame(&mut tls,&Frame::Ready,version).await?;
                    let body=shuttli_transport::receive_body(&mut tls,&meta,||true).await?;
                    if body!=outgoing.data.as_ref() {return Err("outgoing body mismatch".into());}
                    write_live_frame(&mut tls,&Frame::Applied,version).await?;break;
                },other=>return Err(format!("unexpected outgoing frame: {other:?}")),
            }
        }
        let receipt_deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        loop {
        let history=cli(&["history"])?;
        if !history["entries"].as_array().unwrap().iter().any(|h|h["direction"]=="send" && h["state"]=="applied" && h["format"]==serde_json::to_value(format).unwrap()) {if tokio::time::Instant::now()>=receipt_deadline {return Err("outgoing APPLIED history missing".into());} tokio::time::sleep(Duration::from_millis(100)).await;continue;} break; }
        println!("PASS: native Windows {format:?} copy -> TLS peer -> APPLIED outgoing history");
        }
        Ok(())
    });
    let _ = cli(&["set", "send", "off"]);
    if let Some(payload) = saved.payload {
        let before = clipboard.read()?;
        clipboard.write(
            &payload,
            core.authorize_local_copy(before.stamp, payload.meta.clone()),
        )?;
    }
    drop(profile);
    result
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows-only opt-in acceptance driver");
}

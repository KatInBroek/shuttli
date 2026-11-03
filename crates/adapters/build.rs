fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=native/windows/Native.cs");
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let compiler = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
        let helper = out.ancestors().nth(3).unwrap().join("shuttli-windows.exe");
        let source = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("native")
            .join("windows")
            .join("Native.cs");
        let ui_root = source
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("native-ui");
        let ui_source = ui_root.join("windows").join("Desktop.cs");
        let launcher_source = ui_root.join("windows").join("Launcher.cs");
        let manifest = ui_root.join("windows").join("app.manifest");
        let mut resources = vec![(
            ui_root.join("assets").join("app-icon-256.png"),
            "shuttli.AppIcon".to_owned(),
        )];
        resources.push((
            ui_root
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("branding")
                .join("name.txt"),
            "shuttli.ProductName".to_owned(),
        ));
        for locale in ["en", "nl", "de", "fr"] {
            resources.push((
                ui_root.join("locales").join(format!("{locale}.json")),
                format!("shuttli.Locale.{locale}"),
            ));
        }
        for entry in std::fs::read_dir(ui_root.join("assets").join("tray")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "png") {
                resources.push((
                    path.clone(),
                    format!(
                        "shuttli.Tray.{}",
                        path.file_name().unwrap().to_str().unwrap()
                    ),
                ));
            }
        }
        let mut source_bytes = std::fs::read(&source).expect("Windows helper source");
        source_bytes.extend(std::fs::read(&ui_source).expect("Windows UI source"));
        source_bytes.extend(std::fs::read(&launcher_source).expect("Windows launcher source"));
        source_bytes.extend(std::fs::read(&manifest).expect("Windows manifest"));
        source_bytes.extend(std::fs::read("build.rs").expect("build script"));
        source_bytes.extend(std::env::var("CARGO_PKG_VERSION").unwrap().as_bytes());
        println!("cargo:rerun-if-changed={}", ui_source.display());
        println!("cargo:rerun-if-changed={}", launcher_source.display());
        println!("cargo:rerun-if-changed={}", manifest.display());
        for (path, _) in &resources {
            println!("cargo:rerun-if-changed={}", path.display());
            source_bytes.extend(std::fs::read(path).unwrap());
        }
        let marker = helper.with_extension("sources");
        let launcher = helper.with_file_name("shuttli-ui.exe");
        if helper.is_file()
            && launcher.is_file()
            && std::fs::read(&marker).ok().as_ref() == Some(&source_bytes)
        {
            return;
        }
        let compiled = out.join("shuttli-windows.exe");
        let version_source = out.join("Brand.cs");
        std::fs::write(
            &version_source,
            format!(
                "partial class Native {{ const string BuildVersion = \"{}\"; }}",
                std::env::var("CARGO_PKG_VERSION").unwrap()
            ),
        )
        .unwrap();
        let png = std::fs::read(ui_root.join("assets").join("app-icon-256.png")).unwrap();
        let mut icon = vec![0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 32, 0];
        icon.extend((png.len() as u32).to_le_bytes());
        icon.extend(22u32.to_le_bytes());
        icon.extend(png);
        let icon_path = out.join("shuttli.ico");
        std::fs::write(&icon_path, icon).unwrap();
        let mut command = std::process::Command::new(&compiler);
        command.arg(format!("/win32icon:{}", icon_path.display()));
        command.arg(format!("/win32manifest:{}", manifest.display()));
        let framework = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("Microsoft.NET")
            .join("Framework64")
            .join("v4.0.30319");
        for assembly in ["PresentationFramework", "PresentationCore", "WindowsBase"] {
            command.arg(format!(
                "/reference:{}",
                framework
                    .join("WPF")
                    .join(format!("{assembly}.dll"))
                    .display()
            ));
        }
        command.arg(format!(
            "/reference:{}",
            framework.join("System.Xaml.dll").display()
        ));
        for (path, name) in resources {
            command.arg(format!("/resource:{},{}", path.display(), name));
        }
        let status = command
            .args([
                "/nologo",
                "/optimize+",
                "/warnaserror+",
                "/target:exe",
                "/reference:System.Windows.Forms.dll",
                "/reference:System.Drawing.dll",
                "/reference:System.Web.Extensions.dll",
            ])
            .arg(format!("/out:{}", compiled.display()))
            .arg(&source)
            .arg(ui_source)
            .arg(version_source)
            .status()
            .expect(".NET Framework C# compiler is required");
        assert!(status.success(), "Windows helper build failed");
        let launcher_output = out.join("shuttli-ui.exe");
        let status = std::process::Command::new(compiler)
            .args([
                "/nologo",
                "/optimize+",
                "/warnaserror+",
                "/target:winexe",
                "/reference:System.Windows.Forms.dll",
            ])
            .arg(format!("/win32icon:{}", icon_path.display()))
            .arg(format!("/win32manifest:{}", manifest.display()))
            .arg(format!("/out:{}", launcher_output.display()))
            .arg(launcher_source)
            .status()
            .unwrap();
        assert!(status.success(), "Windows launcher build failed");
        std::fs::copy(launcher_output, launcher)
            .expect("stop the desktop launcher before rebuilding");
        std::fs::copy(compiled, &helper)
            .expect("stop the running Windows helper before rebuilding changed native sources");
        std::fs::write(marker, source_bytes).expect("Windows helper build marker");
    }
    let (source, framework, name) = ("Clipboard.swift", "AppKit", "shuttli-pasteboard");
    println!("cargo:rerun-if-changed=native/macos/{source}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join(name);
        let result = std::process::Command::new("swiftc")
            .args(["-O", "-framework", framework])
            .arg(format!("native/macos/{source}"))
            .arg("-o")
            .arg(out)
            .status()
            .expect("swiftc from Xcode Command Line Tools is required");
        assert!(result.success(), "{name} helper build failed");
    }
}

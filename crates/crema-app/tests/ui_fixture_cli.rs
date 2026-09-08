use crema_image::{CancelToken, DecodeLimits, DecodeOutcome, Decoder, Fact, Icc, PreviewSize};
use std::{fs, process::Command};

#[test]
fn generator_is_deterministic_create_only_and_profiled() {
    let root = std::env::temp_dir().join(format!("crema-ui-fixture-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let first = root.join("first.jpg");
    let second = root.join("second.jpg");
    for path in [&first, &second] {
        assert!(
            Command::new(env!("CARGO_BIN_EXE_crema-ui-fixture"))
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_crema-ui-fixture"))
            .arg(&first)
            .status()
            .unwrap()
            .success()
    );
    let decoder = Decoder::new(std::env::current_exe().unwrap(), DecodeLimits::default());
    let DecodeOutcome::Decoded(decoded) = decoder.decode(
        &first,
        crema_image::CandidateFormat::Raster(crema_image::RasterFormat::Jpeg),
        PreviewSize::new(960).unwrap(),
        &CancelToken::new(),
    ) else {
        panic!("generated JPEG must decode");
    };
    assert_eq!(decoded.metadata.dimensions, [960, 640]);
    assert_eq!(decoded.metadata.icc, Fact::Known(Icc::AppliedToSrgb));
    let library = root.join("library");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_crema-ui-fixture"))
            .args(["--count", "32"])
            .arg(&library)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read_dir(&library).unwrap().count(), 32);
    assert_eq!(
        fs::read(library.join("photo-00031.jpg")).unwrap(),
        fs::read(&first).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

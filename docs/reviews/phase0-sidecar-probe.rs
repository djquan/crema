#[path = "../../crates/crema-core/src/edit.rs"]
mod edit;
#[path = "../../crates/crema-core/src/sidecar.rs"]
mod sidecar;

use edit::{EditCommand, EditSession, ExposureCentistops};
use sidecar::{SidecarLocator, SidecarNaming, SidecarStore};

fn classify_sidecar(path: &std::path::Path) -> Option<SidecarNaming> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "raf" | "orf" => Some(SidecarNaming::ReplaceOriginalExtension),
        _ => None,
    }
}

fn main() {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    std::fs::create_dir(&root).unwrap();
    let raf = root.join("photo.RAF");
    let orf = root.join("photo.ORF");
    std::fs::write(&raf, b"RAF source unchanged").unwrap();
    std::fs::write(&orf, b"ORF source unchanged").unwrap();
    let location = |path: &std::path::Path| {
        SidecarLocator::new(
            path,
            SidecarNaming::ReplaceOriginalExtension,
            classify_sidecar,
        )
    };
    let mut first = EditSession::open(SidecarStore.open(location(&raf)).unwrap());
    first.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    let completion = SidecarStore.commit(first.begin_save().unwrap().unwrap());
    first.accept_save(completion);
    let mut second = EditSession::open(SidecarStore.open(location(&orf)).unwrap());
    println!(
        "ORF inherited {} centistops",
        second.recipe().exposure().value()
    );
    assert_eq!(second.recipe().exposure().value(), 100);
    second.apply(EditCommand::SetExposure(
        ExposureCentistops::new(-200).unwrap(),
    ));
    let completion = SidecarStore.commit(second.begin_save().unwrap().unwrap());
    second.accept_save(completion);
    let reopened = EditSession::open(SidecarStore.open(location(&raf)).unwrap());
    println!(
        "RAF reopened at {} centistops",
        reopened.recipe().exposure().value()
    );
    assert_eq!(reopened.recipe().exposure().value(), -200);
    assert_eq!(std::fs::read(&raf).unwrap(), b"RAF source unchanged");
    assert_eq!(std::fs::read(&orf).unwrap(), b"ORF source unchanged");
}

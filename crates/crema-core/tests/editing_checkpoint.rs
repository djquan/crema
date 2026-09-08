use crema_core::edit::{
    EditCommand, EditRecipe, EditRevision, EditSession, ExposureCentistops, SaveState,
};
use crema_core::sidecar::{
    ConflictKind, SaveFailure, SidecarBlockReason, SidecarLocation, SidecarLocator, SidecarNaming,
    SidecarStore,
};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const GOLDEN_XMP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="2" crema:sourceFileName="photo.JPG" crema:exposureCentistops="35"/>
  </rdf:RDF>
</x:xmpmeta>
"#;

const LEGACY_XMP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
  <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
    <rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35"/>
  </rdf:RDF>
</x:xmpmeta>
"#;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("crema-{label}-{}-{sequence}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn classify_sidecar(path: &Path) -> Option<SidecarNaming> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "raf" | "orf" => Some(SidecarNaming::ReplaceOriginalExtension),
        "jpg" | "jpeg" | "dng" => Some(SidecarNaming::AppendXmpExtension),
        _ => None,
    }
}

fn location(directory: &TestDirectory, name: &str, naming: SidecarNaming) -> SidecarLocator {
    SidecarLocator::new(&directory.path().join(name), naming, classify_sidecar)
}

fn sidecar_path(directory: &TestDirectory, name: &str, naming: SidecarNaming) -> PathBuf {
    SidecarLocation::for_original(&directory.path().join(name), naming)
        .unwrap()
        .sidecar()
        .to_owned()
}

fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn exposure_bounds_recipe_revision_and_no_op_are_exact() {
    assert_eq!(ExposureCentistops::new(-500).unwrap().value(), -500);
    assert_eq!(ExposureCentistops::new(500).unwrap().value(), 500);
    assert!(ExposureCentistops::new(-501).is_err());
    assert!(ExposureCentistops::new(501).is_err());

    let directory = TestDirectory::new("edit-revision");
    let store = SidecarStore;
    let open = store
        .open(location(
            &directory,
            "photo.JPG",
            SidecarNaming::AppendXmpExtension,
        ))
        .unwrap();
    let mut session = EditSession::open(open);

    assert_eq!(session.revision(), EditRevision::ZERO);
    assert_eq!(session.save_state(), SaveState::Saved);
    assert!(
        session
            .apply(EditCommand::SetExposure(ExposureCentistops::ZERO))
            .is_none()
    );
    assert_eq!(session.revision(), EditRevision::ZERO);

    let changed = session
        .apply(EditCommand::SetExposure(
            ExposureCentistops::new(100).unwrap(),
        ))
        .unwrap();
    assert_eq!(changed.revision().value(), 1);
    assert_eq!(session.save_state(), SaveState::Dirty);
    assert!(
        session
            .apply(EditCommand::SetExposure(
                ExposureCentistops::new(100).unwrap(),
            ))
            .is_none()
    );
    assert_eq!(session.revision().value(), 1);

    session.apply(EditCommand::ResetExposure).unwrap();
    assert_eq!(session.recipe(), &EditRecipe::default());
    assert_eq!(session.revision().value(), 2);
    assert_eq!(session.save_state(), SaveState::Saved);
}

#[test]
fn completed_save_commits_only_its_submitted_snapshot() {
    let directory = TestDirectory::new("stale-save");
    let store = SidecarStore;
    let open = store
        .open(location(
            &directory,
            "photo.JPG",
            SidecarNaming::AppendXmpExtension,
        ))
        .unwrap();
    let mut session = EditSession::open(open);

    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    let command = session.begin_save().unwrap().unwrap();
    assert!(matches!(session.save_state(), SaveState::Saving { .. }));
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(200).unwrap(),
    ));

    session.accept_save(store.commit(command));

    assert_eq!(session.durable_recipe().exposure().value(), 100);
    assert_eq!(session.recipe().exposure().value(), 200);
    assert_eq!(session.save_state(), SaveState::Dirty);
}

#[test]
fn stale_job_completion_is_ignored_and_io_failure_stays_visible() {
    let directory = TestDirectory::new("save-states");
    let photos = directory.path().join("photos");
    fs::create_dir(&photos).unwrap();
    let store = SidecarStore;
    let open = store
        .open(SidecarLocator::new(
            &photos.join("photo.JPG"),
            SidecarNaming::AppendXmpExtension,
            classify_sidecar,
        ))
        .unwrap();
    let mut session = EditSession::open(open);
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    let first = store.commit(session.begin_save().unwrap().unwrap());
    session.accept_save(first.clone());

    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(200).unwrap(),
    ));
    let second = session.begin_save().unwrap().unwrap();
    session.accept_save(first);
    assert!(matches!(session.save_state(), SaveState::Saving { .. }));

    let parked = directory.path().join("photos-parked");
    fs::rename(&photos, &parked).unwrap();
    fs::write(&photos, b"not a directory").unwrap();
    session.accept_save(store.commit(second));
    assert!(matches!(session.save_state(), SaveState::Failed(_)));
    assert_eq!(session.durable_recipe().exposure().value(), 100);
    assert_eq!(session.recipe().exposure().value(), 200);
    fs::remove_file(&photos).unwrap();
    fs::rename(parked, photos).unwrap();
}

#[test]
fn publication_guard_runs_after_temp_sync_and_before_sidecar_publish() {
    let directory = TestDirectory::new("publication-guard");
    let source = directory.path().join("photo.JPG");
    let parked = directory.path().join("photo-original.JPG");
    fs::write(&source, b"original").unwrap();
    let store = SidecarStore;
    let open = store
        .open(SidecarLocator::new(
            &source,
            SidecarNaming::AppendXmpExtension,
            classify_sidecar,
        ))
        .unwrap();
    let mut session = EditSession::open(open);
    let sidecar = session.sidecar_path().to_owned();
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    let command = session.begin_save().unwrap().unwrap();

    let completion = store.commit_guarded(command, || {
        fs::rename(&source, &parked).unwrap();
        fs::write(&source, b"replacement").unwrap();
        Err(SaveFailure::Io {
            path: source.clone(),
            message: "source changed since it was opened".to_owned(),
        })
    });
    session.accept_save(completion);

    assert!(matches!(session.save_state(), SaveState::Failed(_)));
    assert!(!sidecar.exists());
    assert_eq!(fs::read(&parked).unwrap(), b"original");
    assert_eq!(fs::read(&source).unwrap(), b"replacement");
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("crema-tmp")
    }));
}

#[test]
fn writes_the_exact_owned_packet_and_reopens_it() {
    let directory = TestDirectory::new("golden-xmp");
    let source = directory.path().join("photo.JPG");
    let original = b"not really a jpeg, but never writable";
    fs::write(&source, original).unwrap();
    let original_hash = hash(original);
    let sidecar = sidecar_path(&directory, "photo.JPG", SidecarNaming::AppendXmpExtension);
    let store = SidecarStore;
    let mut session = EditSession::open(
        store
            .open(SidecarLocator::new(
                &source,
                SidecarNaming::AppendXmpExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );

    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(35).unwrap(),
    ));
    let command = session.begin_save().unwrap().unwrap();
    session.accept_save(store.commit(command));

    assert_eq!(session.save_state(), SaveState::Saved);
    assert_eq!(fs::read(&sidecar).unwrap(), GOLDEN_XMP);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(hash(&fs::read(&source).unwrap()), original_hash);

    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(-125).unwrap(),
    ));
    let replacement = session.begin_save().unwrap().unwrap();
    session.accept_save(store.commit(replacement));
    assert_eq!(session.save_state(), SaveState::Saved);
    assert!(
        fs::read_to_string(&sidecar)
            .unwrap()
            .contains("crema:exposureCentistops=\"-125\"")
    );
    assert_eq!(fs::read(&source).unwrap(), original);

    let reopened = EditSession::open(
        store
            .open(SidecarLocator::new(
                &source,
                SidecarNaming::AppendXmpExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    assert_eq!(reopened.recipe().exposure().value(), -125);
    assert_eq!(reopened.save_state(), SaveState::Saved);
}

#[test]
fn accepts_prefix_and_whitespace_variations_then_canonicalizes_on_save() {
    let directory = TestDirectory::new("xmp-prefix");
    let location = location(&directory, "photo.JPG", SidecarNaming::AppendXmpExtension);
    let sidecar = sidecar_path(&directory, "photo.JPG", SidecarNaming::AppendXmpExtension);
    fs::write(
        &sidecar,
        br#"<z:xmpmeta xmlns:z="adobe:ns:meta/">
 <r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <r:Description xmlns:e="urn:crema:xmp:edit" r:about="" e:exposureCentistops="35" e:schemaVersion="1" e:owner="Crema"></r:Description>
 </r:RDF>
</z:xmpmeta>"#,
    )
    .unwrap();
    let store = SidecarStore;
    let mut session = EditSession::open(store.open(location).unwrap());
    assert_eq!(session.recipe().exposure().value(), 35);

    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(40).unwrap(),
    ));
    let completion = store.commit(session.begin_save().unwrap().unwrap());
    session.accept_save(completion);

    assert!(
        fs::read(&sidecar)
            .unwrap()
            .starts_with(br#"<?xml version="1.0" encoding="UTF-8"?>"#)
    );
    assert_eq!(session.recipe().exposure().value(), 40);
    assert_eq!(session.save_state(), SaveState::Saved);
}

#[test]
fn refuses_dtd_foreign_unknown_and_newer_packets_without_changing_bytes() {
    let directory = TestDirectory::new("xmp-refusal");
    let cases: [(&str, &[u8], Option<u32>); 4] = [
        (
            "dtd",
            br#"<!DOCTYPE x [<!ENTITY value "35">]><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="&value;"/></rdf:RDF></x:xmpmeta>"#,
            None,
        ),
        (
            "foreign",
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="foreign"/></rdf:RDF></x:xmpmeta>"#,
            None,
        ),
        (
            "unknown",
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35" crema:contrast="2"/></rdf:RDF></x:xmpmeta>"#,
            None,
        ),
        (
            "newer",
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="3" crema:sourceFileName="newer.JPG" crema:exposureCentistops="35"/></rdf:RDF></x:xmpmeta>"#,
            Some(3),
        ),
    ];
    let store = SidecarStore;

    for (label, bytes, newer) in cases {
        let location = location(
            &directory,
            &format!("{label}.JPG"),
            SidecarNaming::AppendXmpExtension,
        );
        fs::write(location.sidecar(), bytes).unwrap();
        let mut session = EditSession::open(store.open(location).unwrap());

        match (session.save_state(), newer) {
            (
                SaveState::ReadOnly(SidecarBlockReason::NewerSchema { found, .. }),
                Some(expected),
            ) => {
                assert_eq!(found, expected)
            }
            (SaveState::ReadOnly(_), None) => {}
            (actual, expected) => {
                panic!("{label}: unexpected state {actual:?}, newer {expected:?}")
            }
        }
        assert_eq!(fs::read(session.sidecar_path()).unwrap(), bytes, "{label}");
        assert!(session.begin_save().is_err());
        assert_eq!(fs::read(session.sidecar_path()).unwrap(), bytes, "{label}");
    }
}

#[test]
fn refuses_duplicate_rdf_structure_without_changing_bytes() {
    let directory = TestDirectory::new("xmp-duplicate-structure");
    let cases: [(&str, &[u8]); 3] = [
        (
            "duplicate-rdf",
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:RDF><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35"/></rdf:RDF><rdf:RDF><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35"/></rdf:RDF></x:xmpmeta>"#,
        ),
        (
            "duplicate-description",
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:RDF><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35"/><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35"/></rdf:RDF></x:xmpmeta>"#,
        ),
        (
            "empty-rdf-sibling",
            br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:RDF/><rdf:RDF><rdf:Description rdf:about="" xmlns:crema="urn:crema:xmp:edit" crema:owner="Crema" crema:schemaVersion="1" crema:exposureCentistops="35"/></rdf:RDF></x:xmpmeta>"#,
        ),
    ];
    let store = SidecarStore;

    for (label, bytes) in cases {
        let location = location(
            &directory,
            &format!("{label}.JPG"),
            SidecarNaming::AppendXmpExtension,
        );
        fs::write(location.sidecar(), bytes).unwrap();
        let mut session = EditSession::open(store.open(location).unwrap());

        assert!(
            matches!(session.save_state(), SaveState::ReadOnly(_)),
            "{label} opened as editable"
        );
        assert_eq!(fs::read(session.sidecar_path()).unwrap(), bytes, "{label}");
        assert!(session.begin_save().is_err());
        assert_eq!(fs::read(session.sidecar_path()).unwrap(), bytes, "{label}");
    }
}

#[test]
fn successful_create_only_publication_removes_its_temporary_file() {
    let directory = TestDirectory::new("create-only-cleanup");
    let store = SidecarStore;
    let mut session = EditSession::open(
        store
            .open(location(
                &directory,
                "photo.JPG",
                SidecarNaming::AppendXmpExtension,
            ))
            .unwrap(),
    );
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(35).unwrap(),
    ));

    let command = session.begin_save().unwrap().unwrap();
    session.accept_save(store.commit(command));

    assert_eq!(session.save_state(), SaveState::Saved);
    let names = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, ["photo.JPG.xmp"]);
}

#[test]
fn derives_raw_and_rendered_sidecar_names() {
    let directory = TestDirectory::new("sidecar-names");
    let raw = location(
        &directory,
        "DSCF0001.ORF",
        SidecarNaming::ReplaceOriginalExtension,
    );
    let dng = location(
        &directory,
        "IMG.0001.DNG",
        SidecarNaming::AppendXmpExtension,
    );
    let jpeg = location(
        &directory,
        "IMG_0001.JPG",
        SidecarNaming::AppendXmpExtension,
    );

    assert_eq!(raw.sidecar().file_name().unwrap(), "DSCF0001.xmp");
    assert_eq!(dng.sidecar().file_name().unwrap(), "IMG.0001.DNG.xmp");
    assert_eq!(jpeg.sidecar().file_name().unwrap(), "IMG_0001.JPG.xmp");
}

#[test]
fn same_stem_raws_are_ambiguous_before_open() {
    let directory = TestDirectory::new("ambiguous-raws");
    let raf = directory.path().join("photo.RAF");
    let orf = directory.path().join("photo.ORF");
    fs::write(&raf, b"raf").unwrap();
    fs::write(&orf, b"orf").unwrap();

    let mut session = EditSession::open(
        SidecarStore
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    assert!(matches!(
        session.save_state(),
        SaveState::ReadOnly(SidecarBlockReason::AmbiguousOriginals { .. })
    ));
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    assert!(session.is_dirty());
    assert!(session.begin_save().is_err());
    assert!(!directory.path().join("photo.xmp").exists());
    assert_eq!(fs::read(raf).unwrap(), b"raf");
    assert_eq!(fs::read(orf).unwrap(), b"orf");
}

#[test]
fn case_varied_raw_stems_are_treated_as_one_sidecar_slot() {
    let directory = TestDirectory::new("case-varied-raws");
    let raf = directory.path().join("photo.RAF");
    let orf = directory.path().join("PHOTO.ORF");
    fs::write(&raf, b"raf").unwrap();
    fs::write(&orf, b"orf").unwrap();

    let session = EditSession::open(
        SidecarStore
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );

    assert!(matches!(
        session.save_state(),
        SaveState::ReadOnly(SidecarBlockReason::AmbiguousOriginals { .. })
    ));
}

#[test]
fn competing_raw_before_publish_blocks_save_and_keeps_draft() {
    let directory = TestDirectory::new("late-raw");
    let raf = directory.path().join("photo.RAF");
    fs::write(&raf, b"raf").unwrap();
    let store = SidecarStore;
    let mut session = EditSession::open(
        store
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    let command = session.begin_save().unwrap().unwrap();
    fs::write(directory.path().join("photo.ORF"), b"orf").unwrap();

    session.accept_save(store.commit(command));

    assert!(matches!(
        session.save_state(),
        SaveState::ReadOnly(SidecarBlockReason::AmbiguousOriginals { .. })
    ));
    assert_eq!(session.recipe().exposure().value(), 100);
    assert_eq!(session.durable_recipe().exposure().value(), 0);
    assert!(!directory.path().join("photo.xmp").exists());
}

#[test]
fn raw_and_jpeg_companions_save_distinct_associated_recipes() {
    let directory = TestDirectory::new("raw-jpeg");
    let raf = directory.path().join("photo.RAF");
    let jpeg = directory.path().join("photo.JPG");
    fs::write(&raf, b"raf").unwrap();
    fs::write(&jpeg, b"jpeg").unwrap();
    let store = SidecarStore;

    for (path, naming, exposure) in [
        (&raf, SidecarNaming::ReplaceOriginalExtension, 100),
        (&jpeg, SidecarNaming::AppendXmpExtension, -200),
    ] {
        let mut session = EditSession::open(
            store
                .open(SidecarLocator::new(path, naming, classify_sidecar))
                .unwrap(),
        );
        session.apply(EditCommand::SetExposure(
            ExposureCentistops::new(exposure).unwrap(),
        ));
        let command = session.begin_save().unwrap().unwrap();
        session.accept_save(store.commit(command));
        assert_eq!(session.save_state(), SaveState::Saved);
    }

    let raw = EditSession::open(
        store
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    let rendered = EditSession::open(
        store
            .open(SidecarLocator::new(
                &jpeg,
                SidecarNaming::AppendXmpExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    assert_eq!(raw.recipe().exposure().value(), 100);
    assert_eq!(rendered.recipe().exposure().value(), -200);
    assert!(directory.path().join("photo.xmp").exists());
    assert!(directory.path().join("photo.JPG.xmp").exists());
}

#[test]
fn conflicting_supported_raw_sidecar_variants_are_ambiguous() {
    let directory = TestDirectory::new("raw-sidecar-variants");
    let raf = directory.path().join("photo.RAF");
    fs::write(&raf, b"raf").unwrap();
    fs::write(directory.path().join("photo.xmp"), LEGACY_XMP).unwrap();
    fs::write(directory.path().join("photo.RAF.xmp"), LEGACY_XMP).unwrap();

    let session = EditSession::open(
        SidecarStore
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    assert!(matches!(
        session.save_state(),
        SaveState::ReadOnly(SidecarBlockReason::AmbiguousSidecars { .. })
    ));
}

#[test]
fn explicit_raw_association_survives_a_later_competing_original() {
    let directory = TestDirectory::new("explicit-raw");
    let raf = directory.path().join("photo.RAF");
    let orf = directory.path().join("photo.ORF");
    fs::write(&raf, b"raf").unwrap();
    let store = SidecarStore;
    let mut session = EditSession::open(
        store
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    session.apply(EditCommand::SetExposure(
        ExposureCentistops::new(100).unwrap(),
    ));
    let command = session.begin_save().unwrap().unwrap();
    session.accept_save(store.commit(command));
    fs::write(&orf, b"orf").unwrap();

    let raf_session = EditSession::open(
        store
            .open(SidecarLocator::new(
                &raf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    let orf_session = EditSession::open(
        store
            .open(SidecarLocator::new(
                &orf,
                SidecarNaming::ReplaceOriginalExtension,
                classify_sidecar,
            ))
            .unwrap(),
    );
    assert_eq!(raf_session.recipe().exposure().value(), 100);
    assert_eq!(raf_session.save_state(), SaveState::Saved);
    assert!(matches!(
        orf_session.save_state(),
        SaveState::ReadOnly(SidecarBlockReason::AssociatedWithOtherOriginal { .. })
    ));
}

#[test]
fn detects_external_creation_removal_and_change() {
    let directory = TestDirectory::new("sidecar-conflict");
    let store = SidecarStore;

    let created_location = location(&directory, "created.JPG", SidecarNaming::AppendXmpExtension);
    let created_path = created_location.sidecar().to_owned();
    let mut created = EditSession::open(store.open(created_location).unwrap());
    created.apply(EditCommand::SetExposure(
        ExposureCentistops::new(10).unwrap(),
    ));
    let command = created.begin_save().unwrap().unwrap();
    let foreign = b"foreign";
    fs::write(&created_path, foreign).unwrap();
    created.accept_save(store.commit(command));
    assert_eq!(
        created.save_state(),
        SaveState::Conflict(ConflictKind::CreatedExternally)
    );
    assert_eq!(fs::read(&created_path).unwrap(), foreign);

    let changed_location = location(&directory, "changed.JPG", SidecarNaming::AppendXmpExtension);
    fs::write(changed_location.sidecar(), LEGACY_XMP).unwrap();
    let changed_path = changed_location.sidecar().to_owned();
    let mut changed = EditSession::open(store.open(changed_location).unwrap());
    changed.apply(EditCommand::SetExposure(
        ExposureCentistops::new(40).unwrap(),
    ));
    let command = changed.begin_save().unwrap().unwrap();
    let replacement = LEGACY_XMP
        .iter()
        .copied()
        .chain(b" ".iter().copied())
        .collect::<Vec<_>>();
    fs::write(&changed_path, &replacement).unwrap();
    changed.accept_save(store.commit(command));
    assert_eq!(
        changed.save_state(),
        SaveState::Conflict(ConflictKind::ChangedExternally)
    );
    assert_eq!(fs::read(&changed_path).unwrap(), replacement);

    let removed_location = location(&directory, "removed.JPG", SidecarNaming::AppendXmpExtension);
    fs::write(removed_location.sidecar(), LEGACY_XMP).unwrap();
    let removed_path = removed_location.sidecar().to_owned();
    let mut removed = EditSession::open(store.open(removed_location).unwrap());
    removed.apply(EditCommand::SetExposure(
        ExposureCentistops::new(40).unwrap(),
    ));
    let command = removed.begin_save().unwrap().unwrap();
    fs::remove_file(&removed_path).unwrap();
    removed.accept_save(store.commit(command));
    assert_eq!(
        removed.save_state(),
        SaveState::Conflict(ConflictKind::RemovedExternally)
    );
    assert!(!removed_path.exists());
}

#[test]
fn rejects_packets_larger_than_sixty_four_kibibytes() {
    let directory = TestDirectory::new("xmp-cap");
    let location = location(&directory, "large.JPG", SidecarNaming::AppendXmpExtension);
    let bytes = vec![b' '; 64 * 1024 + 1];
    fs::write(location.sidecar(), &bytes).unwrap();
    let session = EditSession::open(SidecarStore.open(location).unwrap());
    assert!(matches!(
        session.save_state(),
        SaveState::ReadOnly(SidecarBlockReason::TooLarge { .. })
    ));
    assert_eq!(fs::read(session.sidecar_path()).unwrap(), bytes);
}

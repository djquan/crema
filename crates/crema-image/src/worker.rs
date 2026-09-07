use crate::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    fs::File,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

const REQUEST: &[u8; 8] = b"CREMAREQ";
const RESPONSE: &[u8; 8] = b"CREMARES";
const META_CAP: usize = 64 * 1024;
const STRING_CAP: usize = 4096;

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum DecodeEvent {
    SourceRead(u64),
    WorkerSpawned(u32),
    WorkerReaped(u32),
}

pub struct Decoder {
    executable: PathBuf,
    limits: DecodeLimits,
}

impl Decoder {
    pub fn new(executable: PathBuf, limits: DecodeLimits) -> Self {
        Self { executable, limits }
    }

    pub fn decode(
        &self,
        path: &Path,
        candidate: CandidateFormat,
        size: PreviewSize,
        cancel: &CancelToken,
    ) -> DecodeOutcome {
        if cancel.is_cancelled() {
            return DecodeOutcome::Failed(DecodeError::new(
                FailureClass::Cancelled,
                "decode cancelled",
            ));
        }
        match File::open(path) {
            Ok(mut file) => self.decode_opened(&mut file, candidate, size, cancel, &|_| {}),
            Err(error) => DecodeOutcome::Failed(error.into()),
        }
    }

    pub fn decode_opened(
        &self,
        file: &mut File,
        candidate: CandidateFormat,
        size: PreviewSize,
        cancel: &CancelToken,
        observe: &dyn Fn(DecodeEvent),
    ) -> DecodeOutcome {
        let codec = match candidate {
            CandidateFormat::Raster(RasterFormat::Jpeg) => 0,
            CandidateFormat::Raw(_) => 1,
            CandidateFormat::Raster(RasterFormat::Heic) => 2,
            _ => return DecodeOutcome::Unsupported(format!("{candidate} decoding is not enabled")),
        };
        let result = (|| {
            if cancel.is_cancelled() {
                return Err(DecodeError::new(FailureClass::Cancelled, "decoder stopped"));
            }
            let mut bytes = Vec::new();
            let mut limited = file.take(self.limits.input_bytes.saturating_add(1));
            let mut chunk = [0; 64 * 1024];
            loop {
                if cancel.is_cancelled() {
                    return Err(DecodeError::new(
                        FailureClass::Cancelled,
                        "attempt cancelled",
                    ));
                }
                let count = limited.read(&mut chunk)?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                observe(DecodeEvent::SourceRead(count as u64));
            }
            if bytes.len() as u64 > self.limits.input_bytes {
                return Err(DecodeError::limit("input byte limit exceeded"));
            }
            if codec == 0 {
                let result = crate::decode::jpeg(&bytes, size, &self.limits);
                if cancel.is_cancelled() {
                    return Err(DecodeError::new(
                        FailureClass::Cancelled,
                        "attempt cancelled",
                    ));
                }
                return result;
            }
            let request = encode_request(codec, size, &self.limits, &bytes);
            supervise(
                &self.executable,
                request,
                size,
                &self.limits,
                cancel,
                observe,
            )
        })();
        match result {
            Ok(result) => DecodeOutcome::Decoded(result),
            Err(error) => DecodeOutcome::Failed(error),
        }
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn supervise(
    executable: &Path,
    request: Vec<u8>,
    size: PreviewSize,
    limits: &DecodeLimits,
    cancel: &CancelToken,
    observe: &dyn Fn(DecodeEvent),
) -> Result<DecodeResult, DecodeError> {
    let started = Instant::now();
    if cancel.is_cancelled() {
        return Err(DecodeError::new(
            FailureClass::Cancelled,
            "attempt cancelled",
        ));
    }
    let mut child = ChildGuard(
        Command::new(executable)
            .arg("--crema-decode-worker")
            .env("RAYON_NUM_THREADS", "2")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let pid = child.0.id();
    observe(DecodeEvent::WorkerSpawned(pid));
    let mut stdin = child.0.stdin.take().expect("piped stdin");
    let stdout = child.0.stdout.take().expect("piped stdout");
    let stderr = child.0.stderr.take().expect("piped stderr");
    let writer = thread::spawn(move || stdin.write_all(&request));
    let (sender, receiver) = mpsc::sync_channel(1);
    let max_output = size.edge() as usize * size.edge() as usize * 4 + META_CAP + 32;
    let reader = thread::spawn(move || {
        let result = read_bounded(stdout, max_output);
        let _ = sender.send(result);
    });
    let errors = thread::spawn(move || drain_stderr(stderr));
    let mut output = None;
    let process_result = loop {
        if cancel.is_cancelled() {
            break Err(DecodeError::new(FailureClass::Cancelled, "decoder stopped"));
        }
        if started.elapsed() >= limits.timeout {
            break Err(DecodeError::new(
                FailureClass::Timeout,
                "worker deadline exceeded",
            ));
        }
        if output.is_none() {
            match receiver.try_recv() {
                Ok(Ok(bytes)) => output = Some(bytes),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(DecodeError::protocol("stdout reader stopped"));
                }
            }
        }
        match child.0.try_wait() {
            Ok(Some(status)) if !status.success() => {
                break Err(DecodeError::new(FailureClass::WorkerExited, status));
            }
            Ok(Some(_)) if output.is_some() => break Ok(()),
            Ok(_) => thread::sleep(Duration::from_millis(5)),
            Err(error) => break Err(error.into()),
        }
    };
    if process_result.is_err() {
        let _ = child.0.kill();
    }
    let _ = child.0.wait();
    observe(DecodeEvent::WorkerReaped(pid));
    let write_result = writer
        .join()
        .map_err(|_| DecodeError::protocol("stdin writer panicked"));
    let _ = reader.join();
    let diagnostics = errors.join().unwrap_or_default();
    if let Err(mut error) = process_result {
        if !diagnostics.is_empty() {
            error
                .message
                .push_str(&format!("; {}", String::from_utf8_lossy(&diagnostics)));
        }
        return Err(error);
    }
    write_result??;
    let result = decode_response(&output.expect("successful process has output"), size)?;
    crate::decode::check_dimensions(
        result.metadata.dimensions[0],
        result.metadata.dimensions[1],
        limits.source_pixels,
    )?;
    crate::decode::check_dimensions(
        result.metadata.decoded_dimensions[0],
        result.metadata.decoded_dimensions[1],
        limits.source_pixels,
    )?;
    Ok(result)
}

fn read_bounded(mut input: impl Read, cap: usize) -> Result<Vec<u8>, DecodeError> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > cap {
        return Err(DecodeError::limit("worker output exceeds byte limit"));
    }
    Ok(bytes)
}

fn drain_stderr(mut input: impl Read) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0; 8192];
    while let Ok(count) = input.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let keep = count.min(META_CAP - retained.len());
        retained.extend_from_slice(&buffer[..keep]);
    }
    retained
}

fn encode_request(codec: u8, size: PreviewSize, limits: &DecodeLimits, bytes: &[u8]) -> Vec<u8> {
    let mut request = Vec::with_capacity(bytes.len() + 40);
    request.extend_from_slice(REQUEST);
    request.extend_from_slice(&1u16.to_le_bytes());
    request.extend_from_slice(&[codec, 0]);
    request.extend_from_slice(&size.edge().to_le_bytes());
    request.extend_from_slice(&limits.source_pixels.to_le_bytes());
    request.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    request.extend_from_slice(bytes);
    request
}

pub fn run_worker(mut input: impl Read, mut output: impl Write) -> Result<(), DecodeError> {
    let mut header = [0; 32];
    input.read_exact(&mut header)?;
    let mut reader = WireReader(Cursor::new(header.as_slice()));
    reader.magic(REQUEST)?;
    reader.version()?;
    let codec = reader.u8()?;
    if !matches!(codec, 1 | 2) || reader.u8()? != 0 {
        return Err(DecodeError::protocol("unknown codec or purpose"));
    }
    let size = PreviewSize::new(reader.u32()?)?;
    let source_pixels = reader.u64()?;
    let input_len = reader.u64()?;
    let defaults = DecodeLimits::default();
    if input_len > defaults.input_bytes
        || source_pixels == 0
        || source_pixels > defaults.source_pixels
    {
        return Err(DecodeError::limit("worker request limits exceeded"));
    }
    let bytes = read_bounded(input, input_len as usize)?;
    if bytes.len() as u64 != input_len {
        return Err(DecodeError::protocol("truncated request"));
    }
    let limits = DecodeLimits {
        source_pixels,
        ..defaults
    };
    let result = match codec {
        1 => crate::decode::raw(bytes, size, &limits),
        2 => crate::decode::heif(&bytes, size, &limits),
        _ => unreachable!(),
    };
    let response = encode_response(result)?;
    output.write_all(&response)?;
    output.flush()?;
    Ok(())
}

struct WireWriter(Vec<u8>);
impl WireWriter {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn text(&mut self, value: &str) -> Result<(), DecodeError> {
        if value.len() > STRING_CAP {
            return Err(DecodeError::limit("metadata string exceeds limit"));
        }
        self.u32(value.len() as u32);
        self.0.extend_from_slice(value.as_bytes());
        Ok(())
    }
    fn reason(&mut self, reason: UnknownReason) {
        self.u8(match reason {
            UnknownReason::DecoderDoesNotExpose => 0,
            UnknownReason::MetadataMissing => 1,
            UnknownReason::MetadataInvalid => 2,
            UnknownReason::NotMeasured => 3,
        });
    }
}

struct WireReader<R>(R);
impl<R: Read> WireReader<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut bytes = [0; N];
        self.0
            .read_exact(&mut bytes)
            .map_err(DecodeError::protocol)?;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.bytes::<1>()?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn magic(&mut self, magic: &[u8; 8]) -> Result<(), DecodeError> {
        if &self.bytes::<8>()? != magic {
            return Err(DecodeError::protocol("invalid magic"));
        }
        Ok(())
    }
    fn version(&mut self) -> Result<(), DecodeError> {
        if u16::from_le_bytes(self.bytes()?) != 1 {
            return Err(DecodeError::protocol("unsupported protocol version"));
        }
        Ok(())
    }
    fn text(&mut self) -> Result<String, DecodeError> {
        let len = self.u32()? as usize;
        if len > STRING_CAP {
            return Err(DecodeError::protocol("oversized metadata string"));
        }
        let mut bytes = vec![0; len];
        self.0
            .read_exact(&mut bytes)
            .map_err(DecodeError::protocol)?;
        String::from_utf8(bytes).map_err(DecodeError::protocol)
    }
    fn reason(&mut self) -> Result<UnknownReason, DecodeError> {
        match self.u8()? {
            0 => Ok(UnknownReason::DecoderDoesNotExpose),
            1 => Ok(UnknownReason::MetadataMissing),
            2 => Ok(UnknownReason::MetadataInvalid),
            3 => Ok(UnknownReason::NotMeasured),
            _ => Err(DecodeError::protocol("unknown reason tag")),
        }
    }
    fn end(&mut self) -> Result<(), DecodeError> {
        if self.0.read(&mut [0])? != 0 {
            return Err(DecodeError::protocol("trailing bytes"));
        }
        Ok(())
    }
}

fn encode_response(result: Result<DecodeResult, DecodeError>) -> Result<Vec<u8>, DecodeError> {
    let mut metadata = WireWriter(Vec::new());
    let (tag, pixels) = match result {
        Err(error) => {
            metadata.u8(match error.class {
                FailureClass::Io => 0,
                FailureClass::InvalidInput => 1,
                FailureClass::LimitExceeded => 2,
                FailureClass::Timeout => 3,
                FailureClass::WorkerExited => 4,
                FailureClass::Protocol => 5,
                FailureClass::Codec => 6,
                FailureClass::Cancelled => 7,
            });
            let mut message = error.message;
            while message.len() > STRING_CAP {
                message.pop();
            }
            metadata.text(&message)?;
            (1, Vec::new())
        }
        Ok(result) => {
            metadata.u8(match result.provenance {
                Provenance::JpegDecode => 0,
                Provenance::RawlerDevelopment => 1,
                Provenance::HeifDecode => 2,
            });
            for value in [
                result.preview.width(),
                result.preview.height(),
                result.metadata.dimensions[0],
                result.metadata.dimensions[1],
                result.metadata.decoded_dimensions[0],
                result.metadata.decoded_dimensions[1],
            ] {
                metadata.u32(value);
            }
            match result.metadata.source_bits {
                Fact::Known(value) => {
                    metadata.u8(1);
                    metadata.u8(value);
                }
                Fact::Unknown(reason) => {
                    metadata.u8(0);
                    metadata.reason(reason);
                }
            }
            metadata.u8(result.metadata.decoded_bits);
            match result.metadata.orientation {
                Orientation::Exif(value) => {
                    metadata.u8(1);
                    metadata.u8(value);
                }
                Orientation::ContainerApplied => metadata.u8(2),
                Orientation::Unknown(reason) => {
                    metadata.u8(0);
                    metadata.reason(reason);
                }
            }
            match result.metadata.icc {
                Fact::Known(Icc::Absent) => metadata.u8(1),
                Fact::Known(Icc::PresentNotApplied) => metadata.u8(2),
                Fact::Unknown(reason) => {
                    metadata.u8(0);
                    metadata.reason(reason);
                }
            }
            match result.metadata.nclx {
                Some(values) => {
                    metadata.u8(1);
                    for value in values {
                        metadata.u32(u32::from(value));
                    }
                }
                None => metadata.u8(0),
            }
            metadata.text(&result.metadata.camera_make)?;
            metadata.text(&result.metadata.camera_model)?;
            metadata.text(&result.metadata.limitations)?;
            (0, result.preview.rgba8().to_vec())
        }
    };
    if metadata.0.len() > META_CAP {
        return Err(DecodeError::limit("metadata exceeds limit"));
    }
    let mut response = Vec::new();
    response.extend_from_slice(RESPONSE);
    response.extend_from_slice(&1u16.to_le_bytes());
    response.push(tag);
    response.extend_from_slice(&(metadata.0.len() as u32).to_le_bytes());
    response.extend_from_slice(&(pixels.len() as u64).to_le_bytes());
    response.extend(metadata.0);
    response.extend(pixels);
    Ok(response)
}

fn decode_response(bytes: &[u8], size: PreviewSize) -> Result<DecodeResult, DecodeError> {
    let mut header = WireReader(Cursor::new(bytes));
    header.magic(RESPONSE)?;
    header.version()?;
    let tag = header.u8()?;
    let metadata_len = header.u32()? as usize;
    let pixels_len = header.u64()?;
    if metadata_len > META_CAP || pixels_len > u64::from(size.edge()).pow(2) * 4 {
        return Err(DecodeError::protocol("response lengths exceed limits"));
    }
    if bytes.len() as u64 != 23 + metadata_len as u64 + pixels_len {
        return Err(DecodeError::protocol("response length mismatch"));
    }
    let mut metadata = WireReader(Cursor::new(&bytes[23..23 + metadata_len]));
    if tag == 1 {
        let class = match metadata.u8()? {
            0 => FailureClass::Io,
            1 => FailureClass::InvalidInput,
            2 => FailureClass::LimitExceeded,
            3 => FailureClass::Timeout,
            4 => FailureClass::WorkerExited,
            5 => FailureClass::Protocol,
            6 => FailureClass::Codec,
            7 => FailureClass::Cancelled,
            _ => return Err(DecodeError::protocol("unknown failure class")),
        };
        let message = metadata.text()?;
        metadata.end()?;
        if pixels_len != 0 {
            return Err(DecodeError::protocol("failure has pixels"));
        }
        return Err(DecodeError::new(class, message));
    }
    if tag != 0 {
        return Err(DecodeError::protocol("unknown outcome tag"));
    }
    let provenance = match metadata.u8()? {
        0 => Provenance::JpegDecode,
        1 => Provenance::RawlerDevelopment,
        2 => Provenance::HeifDecode,
        _ => return Err(DecodeError::protocol("unknown provenance")),
    };
    let width = metadata.u32()?;
    let height = metadata.u32()?;
    let dimensions = [metadata.u32()?, metadata.u32()?];
    let decoded_dimensions = [metadata.u32()?, metadata.u32()?];
    let source_bits = match metadata.u8()? {
        0 => Fact::Unknown(metadata.reason()?),
        1 => Fact::Known(metadata.u8()?),
        _ => return Err(DecodeError::protocol("unknown fact tag")),
    };
    let decoded_bits = metadata.u8()?;
    if decoded_bits == 0 || matches!(source_bits, Fact::Known(0)) {
        return Err(DecodeError::protocol("zero sample precision"));
    }
    let orientation = match metadata.u8()? {
        0 => Orientation::Unknown(metadata.reason()?),
        1 => {
            let value = metadata.u8()?;
            if !(1..=8).contains(&value) {
                return Err(DecodeError::protocol("invalid orientation"));
            }
            Orientation::Exif(value)
        }
        2 => Orientation::ContainerApplied,
        _ => return Err(DecodeError::protocol("unknown orientation tag")),
    };
    let icc = match metadata.u8()? {
        0 => Fact::Unknown(metadata.reason()?),
        1 => Fact::Known(Icc::Absent),
        2 => Fact::Known(Icc::PresentNotApplied),
        _ => return Err(DecodeError::protocol("unknown ICC tag")),
    };
    let nclx = match metadata.u8()? {
        0 => None,
        1 => {
            let mut values = [0; 4];
            for value in &mut values {
                *value = u16::try_from(metadata.u32()?).map_err(DecodeError::protocol)?;
            }
            Some(values)
        }
        _ => return Err(DecodeError::protocol("unknown NCLX tag")),
    };
    let camera_make = metadata.text()?;
    let camera_model = metadata.text()?;
    let limitations = metadata.text()?;
    metadata.end()?;
    let preview = PreviewPixels::new(width, height, bytes[23 + metadata_len..].to_vec(), size)?;
    Ok(DecodeResult {
        preview,
        provenance,
        metadata: SourceMetadata {
            dimensions,
            decoded_dimensions,
            source_bits,
            decoded_bits,
            orientation,
            icc,
            nclx,
            camera_make,
            camera_model,
            limitations,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_frame_round_trips_without_catalog_identity() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[100; 18], 2, 3, image::ExtendedColorType::Rgb8)
            .unwrap();
        let size = PreviewSize::new(320).unwrap();
        let original = crate::decode::jpeg(&jpeg, size, &DecodeLimits::default()).unwrap();
        let metadata = original.metadata.clone();
        let pixels = original.preview.rgba8().to_vec();
        let encoded = encode_response(Ok(original)).unwrap();
        let decoded = decode_response(&encoded, size).unwrap();
        assert_eq!(decoded.metadata, metadata);
        assert_eq!(decoded.preview.rgba8(), pixels);
        assert_eq!(decoded.provenance, Provenance::JpegDecode);
        assert!(decode_response(&encoded, PreviewSize::new(1).unwrap()).is_err());
        let request = encode_request(1, size, &DecodeLimits::default(), b"source");
        assert_eq!(request.len(), 32 + b"source".len());
        assert_eq!(&request[32..], b"source");
    }

    #[test]
    fn rejects_truncated_oversized_unknown_and_extra_frames() {
        let size = PreviewSize::new(10).unwrap();
        let valid = encode_response(Err(DecodeError::codec("bad file"))).unwrap();
        assert_eq!(
            decode_response(&valid, size).unwrap_err().class,
            FailureClass::Codec
        );
        for end in 0..valid.len() {
            assert!(decode_response(&valid[..end], size).is_err());
        }
        for (offset, value) in [(0, 0), (8, 2), (10, 99), (11, 255), (15, 255)] {
            let mut broken = valid.clone();
            broken[offset] = value;
            assert_eq!(
                decode_response(&broken, size).unwrap_err().class,
                FailureClass::Protocol
            );
        }
        let mut extra = valid;
        extra.push(0);
        assert_eq!(
            decode_response(&extra, size).unwrap_err().class,
            FailureClass::Protocol
        );
    }

    #[test]
    fn bounded_reader_and_stderr_retention_do_not_grow_without_limit() {
        assert!(read_bounded(Cursor::new(vec![0; 11]), 10).is_err());
        assert_eq!(
            drain_stderr(Cursor::new(vec![1; META_CAP * 3])).len(),
            META_CAP
        );
    }
}

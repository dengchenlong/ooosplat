//! Bounded, read-only GPS inspection. Raw positions stay in private local files.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub const METADATA_LIMIT: usize = 64 * 1024;
pub const SAMPLE_LIMIT: usize = 16;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpsFix {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
    pub horizontal_std: f64,
    pub vertical_std: f64,
}

impl GpsFix {
    fn valid(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
            && self
                .altitude
                .is_none_or(|n| n.is_finite() && (-12_000.0..=100_000.0).contains(&n))
            && self.horizontal_std.is_finite()
            && self.horizontal_std > 0.0
            && self.vertical_std.is_finite()
            && self.vertical_std > 0.0
    }
    /// ECEF intermediate is never used as a Gaussian/preview coordinate.
    fn ecef(&self) -> [f64; 3] {
        let (s, c) = self.latitude.to_radians().sin_cos();
        let (sl, cl) = self.longitude.to_radians().sin_cos();
        let e2 = 6.694_379_990_141_316_5e-3;
        let n = 6_378_137.0 / (1.0 - e2 * s * s).sqrt();
        // Zero is only a computational ellipsoid surface for horizontal search;
        // missing altitude remains None and is never a 3D position observation.
        let h = self.altitude.unwrap_or(0.0);
        [(n + h) * c * cl, (n + h) * c * sl, (n * (1.0 - e2) + h) * s]
    }
    pub fn enu(&self, origin: &Self) -> [f64; 3] {
        let a = self.ecef();
        let b = origin.ecef();
        let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
        let (s, c) = origin.latitude.to_radians().sin_cos();
        let (sl, cl) = origin.longitude.to_radians().sin_cos();
        [
            -sl * x + cl * y,
            -s * cl * x - s * sl * y + c * z,
            c * cl * x + c * sl * y + s * z,
        ]
    }
}

fn rational(text: &str) -> Option<f64> {
    let text = text.trim();
    let value = if let Some((a, b)) = text.split_once('/') {
        let a = a.parse::<f64>().ok()?;
        let b = b.parse::<f64>().ok()?;
        if !a.is_finite() || !b.is_finite() || b == 0.0 {
            return None;
        }
        a / b
    } else {
        text.parse::<f64>().ok()?
    };
    value.is_finite().then_some(value)
}
fn coordinate(text: &str, latitude: bool) -> Option<f64> {
    let text = text.trim();
    let suffix = text.chars().last()?;
    let (body, sign) = match suffix {
        'N' if latitude => (&text[..text.len() - 1], 1.0),
        'S' if latitude => (&text[..text.len() - 1], -1.0),
        'E' if !latitude => (&text[..text.len() - 1], 1.0),
        'W' if !latitude => (&text[..text.len() - 1], -1.0),
        _ => (text, 1.0),
    };
    let parts = body.split(',').map(rational).collect::<Option<Vec<_>>>()?;
    if body.len() != text.len() && parts.first().is_some_and(|v| *v < 0.) {
        return None;
    }
    let result = match parts.as_slice() {
        [d] => *d,
        [d, m] if *d >= 0.0 && (0.0..60.0).contains(m) => d + m / 60.0,
        [d, m, s] if *d >= 0.0 && (0.0..60.0).contains(m) && (0.0..60.0).contains(s) => {
            d + m / 60.0 + s / 3600.0
        }
        _ => return None,
    } * sign;
    (result.abs() <= if latitude { 90.0 } else { 180.0 }).then_some(result)
}

fn from_exif(bytes: Vec<u8>) -> Option<GpsFix> {
    use exif::{In, Tag, Value};
    let exif = exif::Reader::new().read_raw(bytes).ok()?;
    let ascii = |tag| match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Ascii(values) if values.len() == 1 => {
            std::str::from_utf8(values[0].strip_suffix(&[0]).unwrap_or(&values[0])).ok()
        }
        _ => None,
    };
    let dms = |tag, reference, latitude| {
        let Value::Rational(values) = &exif.get_field(tag, In::PRIMARY)?.value else {
            return None;
        };
        if values.len() != 3 || values.iter().any(|v| v.denom == 0) {
            return None;
        }
        coordinate(
            &format!(
                "{},{},{}{}",
                values[0].to_f64(),
                values[1].to_f64(),
                values[2].to_f64(),
                ascii(reference)?
            ),
            latitude,
        )
    };
    if ascii(Tag::GPSMapDatum).is_some_and(|v| {
        !matches!(
            v.trim().to_ascii_uppercase().as_str(),
            "WGS-84" | "WGS84" | "WGS 84"
        )
    }) {
        return None;
    }
    let altitude = if let Some(field) = exif.get_field(Tag::GPSAltitude, In::PRIMARY) {
        let Value::Rational(v) = &field.value else {
            return None;
        };
        if v.len() != 1 || v[0].denom == 0 {
            return None;
        }
        let reference = exif
            .get_field(Tag::GPSAltitudeRef, In::PRIMARY)?
            .value
            .get_uint(0)?;
        match reference {
            0 => Some(v[0].to_f64()),
            1 => Some(-v[0].to_f64()),
            _ => return None,
        }
    } else {
        None
    };
    // DOP is intentionally not interpreted as metres.
    let accuracy = exif
        .get_field(Tag::GPSHPositioningError, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Rational(v) if v.len() == 1 && v[0].denom != 0 => Some(v[0].to_f64()),
            _ => None,
        })
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(10.0)
        .clamp(0.1, 1000.0);
    Some(GpsFix {
        latitude: dms(Tag::GPSLatitude, Tag::GPSLatitudeRef, true)?,
        longitude: dms(Tag::GPSLongitude, Tag::GPSLongitudeRef, false)?,
        altitude,
        horizontal_std: accuracy,
        vertical_std: 20.0,
    })
}

fn xmp_values(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    use quick_xml::{events::Event, name::ResolveResult, NsReader};
    const EXIF: &[u8] = b"http://ns.adobe.com/exif/1.0/";
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut values = BTreeMap::<String, String>::new();
    let mut current = None;
    let insert = |map: &mut BTreeMap<String, String>, key: String, value: String| -> Option<()> {
        if map
            .insert(key, value.clone())
            .is_some_and(|old| old != value)
        {
            None
        } else {
            Some(())
        }
    };
    loop {
        let (ns, event) = reader.read_resolved_event().ok()?;
        let matching = matches!(ns,ResolveResult::Bound(n) if n.as_ref()==EXIF);
        match event.into_owned() {
            Event::Start(e) | Event::Empty(e) => {
                current =
                    matching.then(|| String::from_utf8_lossy(e.local_name().as_ref()).into_owned());
                for a in e.attributes() {
                    let a = a.ok()?;
                    let (ns, key) = reader.resolver().resolve_attribute(a.key);
                    if matches!(ns,ResolveResult::Bound(n) if n.as_ref()==EXIF) {
                        insert(
                            &mut values,
                            String::from_utf8_lossy(key.as_ref()).into_owned(),
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .ok()?
                                .into_owned(),
                        )?;
                    }
                }
            }
            Event::Text(t) => {
                if let Some(key) = &current {
                    insert(&mut values, key.clone(), t.decode().ok()?.into_owned())?;
                }
            }
            Event::End(_) => current = None,
            Event::DocType(_) | Event::GeneralRef(_) => return None,
            Event::Eof => break,
            _ => {}
        }
    }
    Some(values)
}

fn from_xmp(bytes: &[u8]) -> Option<GpsFix> {
    let values = xmp_values(bytes)?;
    if values.get("GPSMapDatum").is_some_and(|v| {
        !matches!(
            v.trim().to_ascii_uppercase().as_str(),
            "WGS-84" | "WGS84" | "WGS 84"
        )
    }) {
        return None;
    }
    let altitude = values
        .get("GPSAltitude")
        .map(|v| {
            let n = rational(v)?;
            if n < 0. {
                return None;
            }
            match values.get("GPSAltitudeRef")?.as_str() {
                "0" => Some(n),
                "1" => Some(-n),
                _ => None,
            }
        })
        .transpose_option()?;
    Some(GpsFix {
        latitude: coordinate(values.get("GPSLatitude")?, true)?,
        longitude: coordinate(values.get("GPSLongitude")?, false)?,
        altitude,
        horizontal_std: values
            .get("GPSHPositioningError")
            .and_then(|s| rational(s))
            .filter(|n| *n > 0.0)
            .unwrap_or(10.0)
            .clamp(0.1, 1000.0),
        vertical_std: 20.0,
    })
}

// Keep malformed optional altitude distinct from an absent altitude.
trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(v) => v.map(Some),
        }
    }
}

fn metadata(path: &Path) -> Option<Vec<(bool, Vec<u8>)>> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut magic = [0; 8];
    file.read_exact(&mut magic).ok()?;
    let mut remaining = METADATA_LIMIT;
    let mut out = Vec::new();
    if magic.starts_with(&[255, 216]) {
        file.seek(SeekFrom::Start(2)).ok()?;
        for _ in 0..4096 {
            let mut marker = [0; 2];
            file.read_exact(&mut marker).ok()?;
            if marker[0] != 255 {
                return None;
            }
            while marker[1] == 255 {
                file.read_exact(&mut marker[1..]).ok()?;
            }
            if matches!(marker[1], 0xda | 0xd9) {
                return Some(out);
            }
            if marker[1] == 1 || (0xd0..=0xd7).contains(&marker[1]) {
                continue;
            }
            let mut size = [0; 2];
            file.read_exact(&mut size).ok()?;
            let size = u16::from_be_bytes(size).checked_sub(2)? as usize;
            let end = file.stream_position().ok()?.checked_add(size as u64)?;
            if end > length {
                return None;
            }
            if marker[1] == 0xe1 {
                remaining = remaining.checked_sub(size)?;
                let mut b = vec![0; size];
                file.read_exact(&mut b).ok()?;
                if let Some(tiff) = b.strip_prefix(b"Exif\0\0") {
                    out.push((true, tiff.to_vec()));
                } else if let Some(xmp) = b.strip_prefix(b"http://ns.adobe.com/xap/1.0/\0") {
                    out.push((false, xmp.to_vec()));
                } else if b.starts_with(b"http://ns.adobe.com/xmp/extension/") {
                    return None;
                }
            }
            file.seek(SeekFrom::Start(end)).ok()?;
        }
    } else if magic == *b"\x89PNG\r\n\x1a\n" {
        for _ in 0..4096 {
            let mut head = [0; 8];
            file.read_exact(&mut head).ok()?;
            let size = u32::from_be_bytes(head[..4].try_into().ok()?) as usize;
            let end = file.stream_position().ok()?.checked_add(size as u64 + 4)?;
            if end > length {
                return None;
            }
            match &head[4..] {
                b"eXIf" | b"iTXt" => {
                    remaining = remaining.checked_sub(size)?;
                    let mut b = vec![0; size];
                    file.read_exact(&mut b).ok()?;
                    let mut crc = [0; 4];
                    file.read_exact(&mut crc).ok()?;
                    let mut check = crc32fast::Hasher::new();
                    check.update(&head[4..]);
                    check.update(&b);
                    if check.finalize() != u32::from_be_bytes(crc) {
                        return None;
                    }
                    if &head[4..] == b"eXIf" {
                        out.push((true, b));
                    } else if let Some(pos) = b.iter().position(|v| *v == 0) {
                        if &b[..pos] != b"XML:com.adobe.xmp" {
                            file.seek(SeekFrom::Start(end)).ok()?;
                            continue;
                        }
                        let compressed = *b.get(pos + 1)?;
                        if *b.get(pos + 2)? != 0 || compressed > 1 {
                            return None;
                        }
                        let mut p = pos + 3;
                        for _ in 0..2 {
                            p += b.get(p..)?.iter().position(|n| *n == 0)? + 1;
                        }
                        let xml = if compressed == 1 {
                            let mut xml = Vec::new();
                            flate2::read::ZlibDecoder::new(&b[p..])
                                .take(remaining as u64 + 1)
                                .read_to_end(&mut xml)
                                .ok()?;
                            remaining = remaining.checked_sub(xml.len())?;
                            xml
                        } else {
                            b[p..].to_vec()
                        };
                        out.push((false, xml));
                    }
                }
                b"IEND" => return Some(out),
                _ => {}
            }
            file.seek(SeekFrom::Start(end)).ok()?;
        }
    }
    None
}

/// Metadata failures never turn an otherwise supported image into a GPS error.
pub fn read(path: &Path) -> Option<GpsFix> {
    let mut found: Option<GpsFix> = None;
    for (is_exif, bytes) in metadata(path)? {
        let mut fix = if is_exif {
            let packet = exif::Reader::new().read_raw(bytes.clone()).ok()?;
            if !packet
                .fields()
                .any(|f| f.tag.context() == exif::Context::Gps)
            {
                continue;
            }
            from_exif(bytes)?
        } else {
            let values = xmp_values(&bytes)?;
            if !values.keys().any(|k| k.starts_with("GPS")) {
                continue;
            }
            from_xmp(&bytes)?
        };
        if !fix.valid() {
            return None;
        }
        if let Some(old) = &found {
            if (old.latitude - fix.latitude).abs() > 1e-7
                || (old.longitude - fix.longitude).abs() > 1e-7
                || matches!((old.altitude,fix.altitude),(Some(a),Some(b)) if (a-b).abs()>0.1)
            {
                return None;
            }
            fix.altitude = old.altitude.or(fix.altitude);
            fix.horizontal_std = old.horizontal_std.max(fix.horizontal_std);
        }
        found = Some(fix);
    }
    found
}

pub fn sample_indices(total: usize) -> Vec<usize> {
    let count = total.min(SAMPLE_LIMIT);
    match count {
        0 => vec![],
        1 => vec![0],
        _ => (0..count).map(|i| i * (total - 1) / (count - 1)).collect(),
    }
}
pub fn detect(files: &[PathBuf]) -> bool {
    let sample = sample_indices(files.len());
    std::thread::scope(|scope| {
        let handles = sample
            .chunks(sample.len().div_ceil(4).max(1))
            .map(|indices| scope.spawn(move || indices.iter().any(|i| read(&files[*i]).is_some())))
            .collect::<Vec<_>>();
        handles.into_iter().any(|h| h.join().unwrap_or(false))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tiff() -> Vec<u8> {
        // IFD0 points at a GPS IFD; coordinates include rational DMS and a
        // below-sea-level altitude. Fixtures never need pixel data.
        let mut b = b"II\x2a\0\x08\0\0\0".to_vec();
        b.extend(1u16.to_le_bytes());
        b.extend(0x8825u16.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(26u32.to_le_bytes());
        b.extend([0; 4]);
        b.extend(6u16.to_le_bytes());
        let entries = [
            (1, 2, 2, u32::from_le_bytes(*b"S\0\0\0")),
            (2, 5, 3, 104),
            (3, 2, 2, u32::from_le_bytes(*b"W\0\0\0")),
            (4, 5, 3, 128),
            (5, 1, 1, 1),
            (6, 5, 1, 152),
        ];
        for (tag, kind, count, value) in entries {
            b.extend((tag as u16).to_le_bytes());
            b.extend((kind as u16).to_le_bytes());
            b.extend((count as u32).to_le_bytes());
            b.extend(value.to_le_bytes());
        }
        b.extend([0; 4]);
        for n in [33, 30, 0, 151, 12, 0, 50] {
            b.extend((n as u32).to_le_bytes());
            b.extend(1u32.to_le_bytes());
        }
        b
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], value: &[u8]) {
        out.extend((value.len() as u32).to_be_bytes());
        out.extend(kind);
        out.extend(value);
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(value);
        out.extend(crc.finalize().to_be_bytes());
    }
    fn png(packets: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, value) in packets {
            chunk(&mut bytes, kind, value);
        }
        chunk(&mut bytes, b"IEND", &[]);
        bytes
    }
    fn itxt(xml: &[u8], compressed: bool) -> Vec<u8> {
        use std::io::Write;
        let mut b = b"XML:com.adobe.xmp\0".to_vec();
        b.extend([u8::from(compressed), 0, 0, 0]);
        if compressed {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
            e.write_all(xml).unwrap();
            b.extend(e.finish().unwrap());
        } else {
            b.extend(xml);
        }
        b
    }
    #[test]
    fn jpeg_exif_png_exif_and_compressed_xmp_are_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let data = tiff();
        let mut jpg = vec![255, 216, 255, 225];
        jpg.extend(((data.len() + 8) as u16).to_be_bytes());
        jpg.extend(b"Exif\0\0");
        jpg.extend(&data);
        jpg.extend([255, 217]);
        let xml=xmp("g:GPSLatitude='33,30S' g:GPSLongitude='151,12W' g:GPSAltitude='50/1' g:GPSAltitudeRef='1'");
        for (name, bytes) in [
            ("中文.jpg", jpg),
            ("e.png", png(&[(*b"eXIf", data)])),
            ("x.png", png(&[(*b"iTXt", itxt(&xml, false))])),
            ("z.png", png(&[(*b"iTXt", itxt(&xml, true))])),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, &bytes).unwrap();
            let fix = read(&path).unwrap();
            assert_eq!(
                (fix.latitude, fix.longitude, fix.altitude),
                (-33.5, -151.2, Some(-50.))
            );
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn conflicts_corruption_and_decompression_budget_reject_gps_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gps.png");
        let xml = xmp("g:GPSLatitude='0' g:GPSLongitude='0'");
        let bad = xmp("g:GPSLatitude='1/0' g:GPSLongitude='0'");
        for packets in [
            vec![(*b"eXIf", tiff()), (*b"iTXt", itxt(&xml, false))],
            vec![(*b"iTXt", itxt(&xml, false)), (*b"iTXt", itxt(&bad, false))],
            vec![(*b"iTXt", itxt(&vec![b' '; METADATA_LIMIT + 1], true))],
        ] {
            std::fs::write(&path, png(&packets)).unwrap();
            assert!(read(&path).is_none());
        }
        let mut bytes = png(&[(*b"iTXt", itxt(&xml, false))]);
        bytes[15] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(read(&path).is_none());
    }
    #[test]
    fn bounded_sample_can_miss_but_generation_reads_all_and_obeys_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let xml = xmp("g:GPSLatitude='0' g:GPSLongitude='0'");
        let files: Vec<_> = (0..100)
            .map(|i| dir.path().join(format!("{i}.png")))
            .collect();
        for (i, p) in files.iter().enumerate() {
            std::fs::write(
                p,
                png(&if i == 1 {
                    vec![(*b"iTXt", itxt(&xml, false))]
                } else {
                    vec![]
                }),
            )
            .unwrap();
        }
        assert!(!detect(&files));
        let token = tokio_util::sync::CancellationToken::new();
        let inputs: Vec<_> = files
            .into_iter()
            .enumerate()
            .map(|(i, p)| (i.to_string(), p))
            .collect();
        assert_eq!(
            crate::reconstruction::gps::collect(&inputs, &token)
                .unwrap()
                .len(),
            1
        );
        token.cancel();
        assert!(matches!(
            crate::reconstruction::gps::collect(&inputs, &token),
            Err(crate::error::SplatError::Cancelled)
        ));
    }
    fn xmp(fields: &str) -> Vec<u8> {
        format!("<x xmlns:g='http://ns.adobe.com/exif/1.0/' {fields}/>").into_bytes()
    }
    #[test]
    fn fractions_coordinates_and_missing_height() {
        let fix=from_xmp(&xmp("g:GPSLatitude='33,30S' g:GPSLongitude='151,12E' g:GPSAltitude='100/2' g:GPSAltitudeRef='1'")).unwrap();
        assert_eq!(
            (fix.latitude, fix.longitude, fix.altitude),
            (-33.5, 151.2, Some(-50.0))
        );
        assert!(from_xmp(&xmp("g:GPSLatitude='0' g:GPSLongitude='0'"))
            .unwrap()
            .altitude
            .is_none());
        for bad in ["1/0", "NaN", "1/2oops", "1/2/3"] {
            assert!(rational(bad).is_none());
        }
        assert!(coordinate("91N", true).is_none());
        assert!(coordinate("12,60E", false).is_none());
    }
    #[test]
    fn namespaces_and_conflicts() {
        let bytes=b"<r xmlns:a='http://ns.adobe.com/exif/1.0/'><a:GPSLatitude>51,30N</a:GPSLatitude><a:GPSLongitude>0,6W</a:GPSLongitude></r>";
        assert_eq!(from_xmp(bytes).unwrap().latitude, 51.5);
        assert!(from_xmp(b"<!DOCTYPE x><x/>").is_none());
        assert!(from_xmp(b"<x xmlns:g='http://ns.adobe.com/exif/1.0/' g:GPSLatitude='0'><g:GPSLatitude>1</g:GPSLatitude></x>").is_none());
    }
    #[test]
    fn sample_is_bounded_and_spread() {
        for n in [0, 1, 2, 15, 16, 17, 100, 1000] {
            let v = sample_indices(n);
            assert_eq!(v.len(), n.min(16));
            if n > 1 {
                assert_eq!(v[0], 0);
                assert_eq!(*v.last().unwrap(), n - 1);
                assert!(v.windows(2).all(|w| w[0] < w[1]));
            }
        }
    }
    #[test]
    fn enu_handles_dateline_and_metre_axes() {
        let a = GpsFix {
            latitude: 0.0,
            longitude: 179.9999,
            altitude: Some(0.0),
            horizontal_std: 10.0,
            vertical_std: 20.0,
        };
        let mut b = a.clone();
        b.longitude = -179.9999;
        let d = b.enu(&a);
        assert!((d[0] - 22.264).abs() < 0.01);
        assert!(d[1].abs() < 1e-6);
        assert_eq!(a.enu(&a), [0.0; 3]);
    }
}

use bytes::{BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use bytes::Buf;

pub const SOH: u8 = 0x01; // ASCII control-A

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixMsgType {
    Logon,          // 35=A
    Heartbeat,      // 35=0
    TestRequest,    // 35=1
    Logout,         // 35=5
    ResendRequest,  // 35=2
    SequenceReset,  // 35=4
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct FixMessage {
    pub begin_string: String, // 8
    pub body_length: usize,   // 9 (computed/validated)
    pub msg_type: FixMsgType, // 35
    pub fields: HashMap<u32, String>,
}

impl FixMessage {
    pub fn new(msg_type: FixMsgType) -> Self {
        Self {
            begin_string: "FIX.4.4".to_string(),
            body_length: 0,
            msg_type,
            fields: HashMap::new(),
        }
    }

    pub fn set_field(&mut self, tag: u32, value: impl Into<String>) {
        self.fields.insert(tag, value.into());
    }
}

fn compute_checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes.iter().map(|b| *b as u32).sum();
    (sum % 256) as u8
}

fn msg_type_to_str(mt: &FixMsgType) -> &str {
    match mt {
        FixMsgType::Logon => "A",
        FixMsgType::Heartbeat => "0",
        FixMsgType::TestRequest => "1",
        FixMsgType::Logout => "5",
        FixMsgType::ResendRequest => "2",
        FixMsgType::SequenceReset => "4",
        FixMsgType::Unknown(s) => s.as_str(),
    }
}

pub fn msg_type_as_str(mt: &FixMsgType) -> &'static str {
    match mt {
        FixMsgType::Logon => "A",
        FixMsgType::Heartbeat => "0",
        FixMsgType::TestRequest => "1",
        FixMsgType::Logout => "5",
        FixMsgType::ResendRequest => "2",
        FixMsgType::SequenceReset => "4",
        FixMsgType::Unknown(_) => "?",
    }
}

fn parse_msg_type(s: &str) -> FixMsgType {
    match s {
        "A" => FixMsgType::Logon,
        "0" => FixMsgType::Heartbeat,
        "1" => FixMsgType::TestRequest,
        "5" => FixMsgType::Logout,
        "2" => FixMsgType::ResendRequest,
        "4" => FixMsgType::SequenceReset,
        other => FixMsgType::Unknown(other.to_string()),
    }
}

pub fn encode(mut msg: FixMessage) -> Bytes {
    // Build header and body first without checksum to compute 9 and 10
    let mut body = BytesMut::with_capacity(256);

    // Required standard header fields inside body: 35
    // Other fields are appended from msg.fields (excluding 8,9,10)
    body.extend_from_slice(b"35=");
    body.extend_from_slice(msg_type_to_str(&msg.msg_type).as_bytes());
    body.put_u8(SOH);

    // Append all user fields sorted by tag for determinism (not required, but helpful)
    let mut tags: Vec<_> = msg.fields.keys().copied().collect();
    tags.sort_unstable();
    for tag in tags {
        if tag == 8 || tag == 9 || tag == 10 || tag == 35 { continue; }
        body.extend_from_slice(tag.to_string().as_bytes());
        body.put_u8(b'=');
        if let Some(v) = msg.fields.get(&tag) {
            body.extend_from_slice(v.as_bytes());
        }
        body.put_u8(SOH);
    }

    // After body is ready, compute BodyLength as number of bytes after tag 9 field up to and including last SOH before tag 10
    let body_length = body.len();

    let mut out = BytesMut::with_capacity(32 + body_length + 16);
    // 8=BeginString
    out.extend_from_slice(b"8=");
    out.extend_from_slice(msg.begin_string.as_bytes());
    out.put_u8(SOH);
    // 9=BodyLength
    out.extend_from_slice(b"9=");
    out.extend_from_slice(body_length.to_string().as_bytes());
    out.put_u8(SOH);
    // Body
    out.extend_from_slice(&body);

    // 10=CheckSum (sum of all bytes up to and including the SOH before 10=)
    let cks = compute_checksum(&out);
    out.extend_from_slice(b"10=");
    out.extend_from_slice(format!("{:03}", cks).as_bytes());
    out.put_u8(SOH);

    out.freeze()
}

pub fn decode(buf: &[u8]) -> Result<FixMessage, String> {
    // Expect tag-value fields delimited by SOH
    // Must contain 8,9,35,...,10 in correct positions and checksum must verify
    // Find last field 10
    if !buf.ends_with(&[SOH]) { return Err("message must end with SOH".into()); }

    // Verify checksum
    let without_trailer = &buf[..buf.len() - 1]; // strip last SOH for parsing convenience
    let mut fields: Vec<&[u8]> = without_trailer.split(|b| *b == SOH).collect();
    let trailer = fields.last().ok_or("empty message")?;
    let trailer_str = std::str::from_utf8(trailer).map_err(|_| "non-utf8 trailer")?;
    if !trailer_str.starts_with("10=") { return Err("missing 10= trailer".into()); }
    let expected_ck = trailer_str[3..].parse::<u8>().map_err(|_| "bad checksum value")?;

    let checksum_region_end = buf.len() - (trailer.len() + 1); // include SOH before 10
    let actual_ck = compute_checksum(&buf[..checksum_region_end]);
    if actual_ck != expected_ck {
        return Err(format!("checksum mismatch: expected {:03}, got {:03}", expected_ck, actual_ck));
    }

    // Remove trailer from fields list
    fields.pop();

    // Parse fields into map
    let mut map: HashMap<u32, String> = HashMap::new();
    for f in fields.iter() {
        let s = std::str::from_utf8(f).map_err(|_| "non-utf8 field")?;
        let mut it = s.splitn(2, '=');
        let tag = it.next().ok_or("missing tag")?;
        let val = it.next().ok_or("missing value")?;
        let tag_num: u32 = tag.parse().map_err(|_| "non-numeric tag")?;
        map.insert(tag_num, val.to_string());
    }

    // Validate header fields
    let begin_string = map.get(&8).cloned().ok_or("missing 8=BeginString")?;
    let body_len_str = map.get(&9).cloned().ok_or("missing 9=BodyLength")?;
    let msg_type_str = map.get(&35).cloned().ok_or("missing 35=MsgType")?;

    // Validate body length: recompute by re-encoding body from the first field after 9 up to before 10
    // We approximate by subtracting size of 8 and 9 fields plus their SOH from the pre-trailer portion length
    // Note: For strict validation, we would need precise offsets. Here we trust the split order preserved.
    let mut seen_8 = false; let mut seen_9 = false; let mut body_counted = 0usize;
    for f in fields.iter() {
        let s = std::str::from_utf8(f).unwrap();
        if !seen_8 {
            if s.starts_with("8=") { seen_8 = true; continue; }
        }
        if !seen_9 {
            if s.starts_with("9=") { seen_9 = true; continue; }
            continue;
        }
        // from first non-9 field after encountering 9 until before 10
        body_counted += f.len() + 1; // plus SOH
    }
    let body_len_val: usize = body_len_str.parse().map_err(|_| "invalid 9 value")?;
    if body_counted != body_len_val { return Err(format!("BodyLength mismatch: header={} computed={}", body_len_val, body_counted)); }

    let msg_type = parse_msg_type(&msg_type_str);

    // Remove header fields (8,9,35) from map to leave only application fields
    map.remove(&8); map.remove(&9); map.remove(&35);

    Ok(FixMessage { begin_string, body_length: body_len_val, msg_type, fields: map })
}

// Stream framer: extract one full FIX message if present
use bytes::BytesMut as _BytesMut; // alias to avoid confusion above
pub fn try_extract_one(buffer: &mut _BytesMut) -> Option<Bytes> {
    // Look for 8=
    let data: &[u8] = buffer.as_ref();
    let start = find_field_start(data, b"8=")?;
    // Find end of 9= field to know where body starts
    let nine_pos = find_field_start(&data[start..], b"9=")? + start;
    let nine_end = memchr::memchr(SOH, &data[nine_pos..]).map(|i| i + nine_pos)?;
    let body_len_str = std::str::from_utf8(&data[nine_pos + 2..nine_end]).ok()?;
    let body_len: usize = body_len_str.parse().ok()?;
    // Body starts after nine_end+1
    let body_start = nine_end + 1;
    // Trailer length is fixed: "10=" + 3 digits + SOH = 7
    let total_len = body_start + body_len + 7 - start;
    if start + total_len > data.len() { return None; }
    let msg_bytes = Bytes::copy_from_slice(&data[start..start + total_len]);
    // Advance buffer
    buffer.advance(start + total_len);
    Some(msg_bytes)
}

fn find_field_start(haystack: &[u8], pat: &[u8]) -> Option<usize> {
    memchr::memmem::find(haystack, pat)
}

// Convenience constructors for admin messages
pub fn build_logon(heart_bt_int_secs: u32, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::Logon);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(108, heart_bt_int_secs.to_string());
    msg
}

pub fn build_heartbeat(test_req_id: Option<&str>, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::Heartbeat);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    if let Some(id) = test_req_id { msg.set_field(112, id); }
    msg
}

pub fn build_test_request(id: &str, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::TestRequest);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(112, id);
    msg
}

pub fn build_logout(text: Option<&str>, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::Logout);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    if let Some(t) = text { msg.set_field(58, t); }
    msg
}

pub fn build_resend_request(begin_seq_no: u32, end_seq_no: u32, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::ResendRequest);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(7, begin_seq_no.to_string());
    msg.set_field(16, end_seq_no.to_string());
    msg
}

pub fn build_sequence_reset(new_seq_no: u32, gap_fill: bool, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::SequenceReset);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(36, new_seq_no.to_string());
    if gap_fill { msg.set_field(123, "Y"); }
    msg
}
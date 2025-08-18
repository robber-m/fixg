use bytes::Buf;
use bytes::{BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use std::io::{self, Write};

pub const SOH: u8 = 0x01; // ASCII control-A

/// FIX message types as defined in the FIX protocol specification.
///
/// Represents the different types of messages that can be sent and received
/// in a FIX session, corresponding to the MsgType (tag 35) field values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixMsgType {
    /// Logon message (35=A) - initiates a FIX session
    Logon,
    /// Heartbeat message (35=0) - maintains session connectivity
    Heartbeat,
    /// Test Request message (35=1) - requests a heartbeat response
    TestRequest,
    /// Logout message (35=5) - terminates a FIX session
    Logout,
    /// Resend Request message (35=2) - requests retransmission of messages
    ResendRequest,
    /// Sequence Reset message (35=4) - resets message sequence numbers
    SequenceReset,
    /// Unknown or unsupported message type
    Unknown(String),
}

/// Represents a parsed FIX message with its constituent fields.
///
/// This structure contains the standard FIX message header fields and
/// a map of all additional fields present in the message message.
#[derive(Debug, Clone)]
pub struct FixMessage {
    /// FIX protocol version (tag 8) - e.g., "FIX.4.4"
    pub begin_string: String,
    /// Message body length (tag 9) - computed and validated during parsing
    pub body_length: usize,
    /// Message type (tag 35) - determines the message's purpose
    pub msg_type: FixMsgType,
    /// All message fields as tag-value pairs (excluding standard header/trailer)
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

/// Encodes a FIX message to bytes
pub fn encode(msg: &FixMessage) -> Result<Bytes, String> {
    let mut buffer = Vec::new();
    encode_to_writer(msg, &mut buffer)?;
    Ok(Bytes::from(buffer))
}

/// Encodes a FIX message to a writer (zero-allocation when reusing buffer)
pub fn encode_to_writer<W: Write>(msg: &FixMessage, writer: &mut W) -> Result<(), String> {
    // Calculate body length (excluding BeginString, BodyLength, and CheckSum)
    let mut body_fields = Vec::new();

    // Add MsgType first
    body_fields.push((35, msg_type_as_str(&msg.msg_type)));

    // Add other fields (sorted by tag number for consistency)
    let mut sorted_fields: Vec<_> = msg.fields.iter().collect();
    sorted_fields.sort_by_key(|(tag, _)| *tag);

    for (tag, value) in sorted_fields {
        body_fields.push((*tag, value.as_str()));
    }

    // Calculate body length
    let body_length: usize = body_fields.iter()
        .map(|(tag, value)| tag.to_string().len() + 1 + value.len() + 1) // tag=value\x01
        .sum();

    // Write BeginString
    write!(writer, "8=FIX.4.4{}", SOH as char).map_err(|e| e.to_string())?;

    // Write BodyLength
    write!(writer, "9={}{}", body_length, SOH as char).map_err(|e| e.to_string())?;

    // Write body fields
    for (tag, value) in body_fields {
        write!(writer, "{}={}{}", tag, value, SOH as char).map_err(|e| e.to_string())?;
    }

    // Calculate checksum by re-creating the message up to this point
    let mut temp_buffer = Vec::new();
    write!(temp_buffer, "8=FIX.4.4{}", SOH as char).unwrap();
    write!(temp_buffer, "9={}{}", body_length, SOH as char).unwrap();
    for (tag, value) in &body_fields {
        write!(temp_buffer, "{}={}{}", tag, value, SOH as char).unwrap();
    }

    let checksum = temp_buffer.iter().fold(0u8, |acc, &b| acc.wrapping_add(b)) % 256;
    write!(writer, "10={:03}{}", checksum, SOH as char).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn decode(buf: &[u8]) -> Result<FixMessage, String> {
    // Expect tag-value fields delimited by SOH
    // Must contain 8,9,35,...,10 in correct positions and checksum must verify
    // Find last field 10
    if !buf.ends_with(&[SOH]) {
        return Err("message must end with SOH".into());
    }

    // Verify checksum
    let without_trailer = &buf[..buf.len() - 1]; // strip last SOH for parsing convenience
    let mut fields: Vec<&[u8]> = without_trailer.split(|b| *b == SOH).collect();
    let trailer = fields.last().ok_or("empty message")?;
    let trailer_str = std::str::from_utf8(trailer).map_err(|_| "non-utf8 trailer")?;
    if !trailer_str.starts_with("10=") {
        return Err("missing 10= trailer".into());
    }
    let expected_ck = trailer_str[3..]
        .parse::<u8>()
        .map_err(|_| "bad checksum value")?;

    let checksum_region_end = buf.len() - (trailer.len() + 1); // include SOH before 10
    let actual_ck = compute_checksum(&buf[..checksum_region_end]);
    if actual_ck != expected_ck {
        return Err(format!(
            "checksum mismatch: expected {:03}, got {:03}",
            expected_ck, actual_ck
        ));
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
    let mut seen_8 = false;
    let mut seen_9 = false;
    let mut body_counted = 0usize;
    for f in fields.iter() {
        let s = std::str::from_utf8(f).unwrap();
        if !seen_8 {
            if s.starts_with("8=") {
                seen_8 = true;
                continue;
            }
        }
        if !seen_9 {
            if s.starts_with("9=") {
                seen_9 = true;
                continue;
            }
            continue;
        }
        // from first non-9 field after encountering 9 until before 10
        body_counted += f.len() + 1; // plus SOH
    }
    let body_len_val: usize = body_len_str.parse().map_err(|_| "invalid 9 value")?;
    if body_counted != body_len_val {
        return Err(format!(
            "BodyLength mismatch: header={} computed={}",
            body_len_val, body_counted
        ));
    }

    let msg_type = parse_msg_type(&msg_type_str);

    // Remove header fields (8,9,35) from map to leave only application fields
    map.remove(&8);
    map.remove(&9);
    map.remove(&35);

    Ok(FixMessage {
        begin_string,
        body_length: body_len_val,
        msg_type,
        fields: map,
    })
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
    if start + total_len > data.len() {
        return None;
    }
    let msg_bytes = Bytes::copy_from_slice(&data[start..start + total_len]);
    // Advance buffer
    buffer.advance(start + total_len);
    Some(msg_bytes)
}

fn find_field_start(haystack: &[u8], pat: &[u8]) -> Option<usize> {
    memchr::memmem::find(haystack, pat)
}

// Convenience constructors for admin messages
pub fn build_logon(
    heart_bt_int_secs: u32,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::Logon);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(108, heart_bt_int_secs.to_string());
    msg
}

pub fn build_heartbeat(
    test_req_id: Option<&str>,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::Heartbeat);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    if let Some(id) = test_req_id {
        msg.set_field(112, id);
    }
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
    if let Some(t) = text {
        msg.set_field(58, t);
    }
    msg
}

pub fn build_resend_request(
    begin_seq_no: u32,
    end_seq_no: u32,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::ResendRequest);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(7, begin_seq_no.to_string());
    msg.set_field(16, end_seq_no.to_string());
    msg
}

pub fn build_sequence_reset(
    new_seq_no: u32,
    gap_fill: bool,
    sender_comp_id: &str,
    target_comp_id: &str,
) -> FixMessage {
    let mut msg = FixMessage::new(FixMsgType::SequenceReset);
    msg.set_field(49, sender_comp_id);
    msg.set_field(56, target_comp_id);
    msg.set_field(36, new_seq_no.to_string());
    if gap_fill {
        msg.set_field(123, "Y");
    }
    msg
}
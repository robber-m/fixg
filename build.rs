
use std::env;
use std::fs;
use std::path::Path;
use proc_macro2::TokenStream;
use quote::quote;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct FixDictionary {
    #[serde(rename = "fix")]
    fix: FixRoot,
}

#[derive(Debug, Deserialize)]
struct FixRoot {
    #[serde(rename = "@major")]
    major: String,
    #[serde(rename = "@minor")]
    minor: String,
    header: Header,
    trailer: Trailer,
    messages: Messages,
    fields: Fields,
    components: Option<Components>,
}

#[derive(Debug, Deserialize)]
struct Header {
    field: Vec<FieldRef>,
}

#[derive(Debug, Deserialize)]
struct Trailer {
    field: Vec<FieldRef>,
}

#[derive(Debug, Deserialize)]
struct Messages {
    message: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@msgtype")]
    msgtype: String,
    #[serde(rename = "@msgcat")]
    msgcat: Option<String>,
    field: Option<Vec<FieldRef>>,
    group: Option<Vec<Group>>,
    component: Option<Vec<ComponentRef>>,
}

#[derive(Debug, Deserialize)]
struct Fields {
    field: Vec<Field>,
}

#[derive(Debug, Deserialize)]
struct Field {
    #[serde(rename = "@number")]
    number: u32,
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@type")]
    field_type: String,
    value: Option<Vec<FieldValue>>,
}

#[derive(Debug, Deserialize)]
struct FieldValue {
    #[serde(rename = "@enum")]
    enum_val: String,
    #[serde(rename = "@description")]
    description: String,
}

#[derive(Debug, Deserialize)]
struct FieldRef {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@required")]
    required: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Group {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@required")]
    required: Option<String>,
    field: Option<Vec<FieldRef>>,
}

#[derive(Debug, Deserialize)]
struct Components {
    component: Vec<Component>,
}

#[derive(Debug, Deserialize)]
struct Component {
    #[serde(rename = "@name")]
    name: String,
    field: Option<Vec<FieldRef>>,
    group: Option<Vec<Group>>,
    component: Option<Vec<ComponentRef>>,
}

#[derive(Debug, Deserialize)]
struct ComponentRef {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@required")]
    required: Option<String>,
}

fn generate_basic_messages() -> TokenStream {
    quote! {
        use crate::protocol::{FixMessage, FixMsgType};
        use std::convert::TryFrom;
        use std::collections::HashMap;

        #[derive(Debug, Clone)]
        pub enum AdminMessage {
            Logon {
                heart_bt_int_secs: Option<u32>,
                sender_comp_id: Option<String>,
                target_comp_id: Option<String>,
                encrypt_method: Option<u32>,
                reset_seq_num_flag: Option<bool>,
            },
            Heartbeat { 
                test_req_id: Option<String> 
            },
            TestRequest { 
                test_req_id: String 
            },
            Logout { 
                text: Option<String>,
                session_status: Option<u32>,
            },
            ResendRequest {
                begin_seq_no: u32,
                end_seq_no: u32,
            },
            SequenceReset {
                gap_fill_flag: Option<bool>,
                new_seq_no: u32,
            },
        }

        impl TryFrom<&FixMessage> for AdminMessage {
            type Error = String;
            
            fn try_from(msg: &FixMessage) -> Result<Self, Self::Error> {
                match msg.msg_type {
                    FixMsgType::Logon => {
                        let heart_bt_int = msg.fields.get(&108).and_then(|s| s.parse::<u32>().ok());
                        let sender_comp_id = msg.fields.get(&49).cloned();
                        let target_comp_id = msg.fields.get(&56).cloned();
                        let encrypt_method = msg.fields.get(&98).and_then(|s| s.parse::<u32>().ok());
                        let reset_seq_num_flag = msg.fields.get(&141).and_then(|s| s.parse::<bool>().ok());
                        
                        Ok(AdminMessage::Logon {
                            heart_bt_int_secs: heart_bt_int,
                            sender_comp_id,
                            target_comp_id,
                            encrypt_method,
                            reset_seq_num_flag,
                        })
                    }
                    FixMsgType::Heartbeat => {
                        let test_req_id = msg.fields.get(&112).cloned();
                        Ok(AdminMessage::Heartbeat { test_req_id })
                    }
                    FixMsgType::TestRequest => {
                        let test_req_id = msg.fields.get(&112)
                            .ok_or("TestRequest missing required TestReqID(112)")?
                            .clone();
                        Ok(AdminMessage::TestRequest { test_req_id })
                    }
                    FixMsgType::Logout => {
                        let text = msg.fields.get(&58).cloned();
                        let session_status = msg.fields.get(&1409).and_then(|s| s.parse::<u32>().ok());
                        Ok(AdminMessage::Logout { text, session_status })
                    }
                    FixMsgType::ResendRequest => {
                        let begin_seq_no = msg.fields.get(&7)
                            .ok_or("ResendRequest missing required BeginSeqNo(7)")?
                            .parse::<u32>()
                            .map_err(|_| "Invalid BeginSeqNo")?;
                        let end_seq_no = msg.fields.get(&16)
                            .ok_or("ResendRequest missing required EndSeqNo(16)")?
                            .parse::<u32>()
                            .map_err(|_| "Invalid EndSeqNo")?;
                        Ok(AdminMessage::ResendRequest { begin_seq_no, end_seq_no })
                    }
                    FixMsgType::SequenceReset => {
                        let gap_fill_flag = msg.fields.get(&123).and_then(|s| s.parse::<bool>().ok());
                        let new_seq_no = msg.fields.get(&36)
                            .ok_or("SequenceReset missing required NewSeqNo(36)")?
                            .parse::<u32>()
                            .map_err(|_| "Invalid NewSeqNo")?;
                        Ok(AdminMessage::SequenceReset { gap_fill_flag, new_seq_no })
                    }
                    FixMsgType::Unknown(_) => Err("Unknown message type".to_string()),
                }
            }
        }

        impl AdminMessage {
            pub fn into_fix(self, sender_comp_id: &str, target_comp_id: &str) -> FixMessage {
                let mut msg = match self {
                    AdminMessage::Logon { heart_bt_int_secs, encrypt_method, reset_seq_num_flag, .. } => {
                        let mut m = FixMessage::new(FixMsgType::Logon);
                        if let Some(hb) = heart_bt_int_secs { 
                            m.fields.insert(108, hb.to_string()); 
                        }
                        if let Some(em) = encrypt_method {
                            m.fields.insert(98, em.to_string());
                        }
                        if let Some(reset) = reset_seq_num_flag {
                            m.fields.insert(141, if reset { "Y" } else { "N" }.to_string());
                        }
                        m
                    }
                    AdminMessage::Heartbeat { test_req_id } => {
                        let mut m = FixMessage::new(FixMsgType::Heartbeat);
                        if let Some(id) = test_req_id { 
                            m.fields.insert(112, id); 
                        }
                        m
                    }
                    AdminMessage::TestRequest { test_req_id } => {
                        let mut m = FixMessage::new(FixMsgType::TestRequest);
                        m.fields.insert(112, test_req_id);
                        m
                    }
                    AdminMessage::Logout { text, session_status } => {
                        let mut m = FixMessage::new(FixMsgType::Logout);
                        if let Some(t) = text { 
                            m.fields.insert(58, t); 
                        }
                        if let Some(status) = session_status {
                            m.fields.insert(1409, status.to_string());
                        }
                        m
                    }
                    AdminMessage::ResendRequest { begin_seq_no, end_seq_no } => {
                        let mut m = FixMessage::new(FixMsgType::ResendRequest);
                        m.fields.insert(7, begin_seq_no.to_string());
                        m.fields.insert(16, end_seq_no.to_string());
                        m
                    }
                    AdminMessage::SequenceReset { gap_fill_flag, new_seq_no } => {
                        let mut m = FixMessage::new(FixMsgType::SequenceReset);
                        if let Some(gap_fill) = gap_fill_flag {
                            m.fields.insert(123, if gap_fill { "Y" } else { "N" }.to_string());
                        }
                        m.fields.insert(36, new_seq_no.to_string());
                        m
                    }
                };
                
                // Standard header fields
                msg.fields.insert(49, sender_comp_id.to_string());
                msg.fields.insert(56, target_comp_id.to_string());
                msg
            }

            pub fn msg_type(&self) -> FixMsgType {
                match self {
                    AdminMessage::Logon { .. } => FixMsgType::Logon,
                    AdminMessage::Heartbeat { .. } => FixMsgType::Heartbeat,
                    AdminMessage::TestRequest { .. } => FixMsgType::TestRequest,
                    AdminMessage::Logout { .. } => FixMsgType::Logout,
                    AdminMessage::ResendRequest { .. } => FixMsgType::ResendRequest,
                    AdminMessage::SequenceReset { .. } => FixMsgType::SequenceReset,
                }
            }
        }

        // Builder patterns for admin messages
        pub struct LogonBuilder {
            heart_bt_int_secs: Option<u32>,
            encrypt_method: Option<u32>,
            reset_seq_num_flag: Option<bool>,
        }

        impl LogonBuilder {
            pub fn new() -> Self {
                Self {
                    heart_bt_int_secs: None,
                    encrypt_method: Some(0), // No encryption by default
                    reset_seq_num_flag: None,
                }
            }

            pub fn heartbeat_interval(mut self, seconds: u32) -> Self {
                self.heart_bt_int_secs = Some(seconds);
                self
            }

            pub fn encrypt_method(mut self, method: u32) -> Self {
                self.encrypt_method = Some(method);
                self
            }

            pub fn reset_sequence_numbers(mut self, reset: bool) -> Self {
                self.reset_seq_num_flag = Some(reset);
                self
            }

            pub fn build(self) -> AdminMessage {
                AdminMessage::Logon {
                    heart_bt_int_secs: self.heart_bt_int_secs,
                    sender_comp_id: None, // Will be set when converting to FixMessage
                    target_comp_id: None, // Will be set when converting to FixMessage
                    encrypt_method: self.encrypt_method,
                    reset_seq_num_flag: self.reset_seq_num_flag,
                }
            }
        }

        pub struct TestRequestBuilder {
            test_req_id: String,
        }

        impl TestRequestBuilder {
            pub fn new(test_req_id: String) -> Self {
                Self { test_req_id }
            }

            pub fn build(self) -> AdminMessage {
                AdminMessage::TestRequest {
                    test_req_id: self.test_req_id,
                }
            }
        }

        pub struct LogoutBuilder {
            text: Option<String>,
            session_status: Option<u32>,
        }

        impl LogoutBuilder {
            pub fn new() -> Self {
                Self {
                    text: None,
                    session_status: None,
                }
            }

            pub fn text(mut self, text: String) -> Self {
                self.text = Some(text);
                self
            }

            pub fn session_status(mut self, status: u32) -> Self {
                self.session_status = Some(status);
                self
            }

            pub fn build(self) -> AdminMessage {
                AdminMessage::Logout {
                    text: self.text,
                    session_status: self.session_status,
                }
            }
        }

        // Zero-copy message decoder
        pub struct MessageDecoder<'a> {
            data: &'a [u8],
        }

        impl<'a> MessageDecoder<'a> {
            pub fn new(data: &'a [u8]) -> Self {
                Self { data }
            }

            pub fn decode(&self) -> Result<FixMessage, String> {
                crate::protocol::decode(self.data)
                    .map_err(|e| format!("Decode error: {}", e))
            }
        }

        // Zero-allocation message encoder
        pub struct MessageEncoder {
            buffer: Vec<u8>,
        }

        impl MessageEncoder {
            pub fn new() -> Self {
                Self {
                    buffer: Vec::with_capacity(1024),
                }
            }

            pub fn encode(&mut self, msg: &FixMessage) -> Result<bytes::Bytes, String> {
                self.buffer.clear();
                crate::protocol::encode_to_writer(msg, &mut self.buffer)
                    .map_err(|e| format!("Encode error: {}", e))?;
                Ok(bytes::Bytes::copy_from_slice(&self.buffer))
            }

            pub fn encode_admin(&mut self, msg: &AdminMessage, sender_comp_id: &str, target_comp_id: &str) -> Result<bytes::Bytes, String> {
                let fix_msg = msg.clone().into_fix(sender_comp_id, target_comp_id);
                self.encode(&fix_msg)
            }
        }

        impl Default for LogonBuilder {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Default for LogoutBuilder {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Default for MessageEncoder {
            fn default() -> Self {
                Self::new()
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=fix_dictionaries/");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("generated.rs");

    // For now, generate basic admin messages
    // In the future, this would parse XML dictionaries from fix_dictionaries/
    let generated_code = generate_basic_messages();

    fs::write(&dest_path, generated_code.to_string()).unwrap();

    // Copy the generated file to src/messages/ for development
    let src_dest = Path::new("src/messages/generated.rs");
    if let Ok(generated_content) = fs::read_to_string(&dest_path) {
        let header = "// AUTO-GENERATED by build.rs. Do not edit by hand.\n";
        let final_content = format!("{}{}", header, generated_content);
        let _ = fs::write(src_dest, final_content);
    }
}

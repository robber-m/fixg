use bytes::{Bytes, BytesMut};

pub mod generated;
pub use generated::AdminMessage;

/// FIX Logon message structure.
/// 
/// Represents a FIX Logon message used to initiate a session.
/// Currently a placeholder structure for future expansion.
#[derive(Debug, Clone)]
pub struct Logon;

/// Execution report type indicating how an order was processed.
/// 
/// Defines the different ways an order can be executed or processed
/// in the trading system.
#[derive(Debug, Clone)]
pub enum ExecType { 
    /// Order was completely filled
    Fill 
}

/// Current status of an order in the trading system.
/// 
/// Indicates the current state of an order from submission to completion.
#[derive(Debug, Clone)]
pub enum OrdStatus { 
    /// Order has been completely filled
    Filled 
}

/// FIX Execution Report message containing order execution details.
/// 
/// Reports the status and execution details of an order, including
/// pricing, quantity, and execution type information.
#[derive(Debug, Clone)]
pub struct ExecutionReport {
    /// Client-assigned order identifier
    cl_ord_id: String,
    /// Exchange-assigned order identifier
    order_id: String,
    /// Type of execution that occurred
    exec_type: ExecType,
    /// Current status of the order
    ord_status: OrdStatus,
    /// Price of the last fill
    last_px: f64,
    /// Quantity of the last fill
    last_qty: i64,
}

impl ExecutionReport {
    pub fn builder() -> ExecutionReportBuilder { ExecutionReportBuilder::default() }
}

/// Builder pattern implementation for constructing ExecutionReport instances.
/// 
/// Provides a fluent interface for setting execution report fields
/// with optional validation and default values.
#[derive(Debug, Default)]
pub struct ExecutionReportBuilder {
    /// Client order ID being built
    cl_ord_id: Option<String>,
    /// Order ID being built
    order_id: Option<String>,
    /// Execution type being built
    exec_type: Option<ExecType>,
    /// Order status being built
    ord_status: Option<OrdStatus>,
    /// Last execution price being built
    last_px: Option<f64>,
    /// Last execution quantity being built
    last_qty: Option<i64>,
}

impl ExecutionReportBuilder {
    pub fn cl_ord_id(mut self, v: impl Into<String>) -> Self { self.cl_ord_id = Some(v.into()); self }
    pub fn order_id(mut self, v: impl Into<String>) -> Self { self.order_id = Some(v.into()); self }
    pub fn exec_type(mut self, v: ExecType) -> Self { self.exec_type = Some(v); self }
    pub fn ord_status(mut self, v: OrdStatus) -> Self { self.ord_status = Some(v); self }
    pub fn last_px(mut self, v: f64) -> Self { self.last_px = Some(v); self }
    pub fn last_qty(mut self, v: i64) -> Self { self.last_qty = Some(v); self }

    pub fn build(self) -> ExecutionReport {
        ExecutionReport {
            cl_ord_id: self.cl_ord_id.unwrap_or_default(),
            order_id: self.order_id.unwrap_or_default(),
            exec_type: self.exec_type.unwrap_or(ExecType::Fill),
            ord_status: self.ord_status.unwrap_or(OrdStatus::Filled),
            last_px: self.last_px.unwrap_or_default(),
            last_qty: self.last_qty.unwrap_or_default(),
        }
    }
}

impl From<ExecutionReport> for Bytes {
    fn from(er: ExecutionReport) -> Self {
        // Placeholder encoding; real impl would encode FIX tags. Here we serialize as a simple string.
        let mut buf = BytesMut::new();
        let s = format!(
            "ExecReport|ClOrdID={}|OrderID={}|LastPx={}|LastQty={}",
            er.cl_ord_id, er.order_id, er.last_px, er.last_qty
        );
        buf.extend_from_slice(s.as_bytes());
        buf.freeze()
    }
}
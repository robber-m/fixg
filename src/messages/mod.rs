use bytes::{Bytes, BytesMut};

#[derive(Debug, Clone)]
pub struct Logon;

#[derive(Debug, Clone)]
pub enum ExecType { Fill }

#[derive(Debug, Clone)]
pub enum OrdStatus { Filled }

#[derive(Debug, Clone)]
pub struct ExecutionReport {
    cl_ord_id: String,
    order_id: String,
    exec_type: ExecType,
    ord_status: OrdStatus,
    last_px: f64,
    last_qty: i64,
}

impl ExecutionReport {
    pub fn builder() -> ExecutionReportBuilder { ExecutionReportBuilder::default() }
}

#[derive(Debug, Default)]
pub struct ExecutionReportBuilder {
    cl_ord_id: Option<String>,
    order_id: Option<String>,
    exec_type: Option<ExecType>,
    ord_status: Option<OrdStatus>,
    last_px: Option<f64>,
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
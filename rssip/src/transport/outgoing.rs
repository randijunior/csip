use core::fmt;
use std::io::Write;
use std::net::SocketAddr;
use std::ops;

use bytes::{BufMut, Bytes, BytesMut};
use utils::encode::Encode;

use crate::message::headers::ContentLength;
use crate::message::uri::HostPort;
use crate::message::{Request, Response, SipBody};
use crate::parser::HeaderParse;
use crate::transport::{TransportHandle, TransportProtocol};

/// This type represents an outbound SIP request.
pub struct OutgoingRequest {
    /// The SIP request.
    pub request: Request,
    /// Metadata about how the message will be sent.
    pub target_info: TargetTransportInfo,
    /// Message encoded representation.
    pub encoded: Bytes,
}

/// This type represents an outgoing SIP response.
pub struct OutgoingResponse {
    /// The SIP response.
    pub response: Response,
    /// Metadata about how the message will be sent.
    pub(crate) dest_info: OutgoingDestInfo,
    /// Message encoded representation.
    pub(crate) encoded: Bytes,
}

/// Outgoing message info.
#[derive(Clone)]
pub struct TargetTransportInfo {
    /// The socket this message should be sent to.
    pub socket_addr: SocketAddr,
    /// The transport to use for sending the message.
    pub transport: TransportHandle,
}

#[derive(Clone)]
pub struct OutgoingDestInfo {
    pub host_port: (HostPort, TransportProtocol),
    pub transport: Option<TargetTransportInfo>,
}

impl fmt::Display for OutgoingDestInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(TargetTransportInfo { socket_addr, .. }) = self.transport {
            write!(f, "{socket_addr}")?;
        } else {
            write!(f, "{}", self.host_port.0.host)?;
        }

        Ok(())
    }
}

impl ops::Deref for OutgoingRequest {
    type Target = Request;
    fn deref(&self) -> &Self::Target {
        &self.request
    }
}

impl ops::DerefMut for OutgoingRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.request
    }
}

impl OutgoingResponse {
    pub fn encoded_str(&self) -> &str {
        // SAFETY: An parsed OutgoingResponse is aways a correctly
        // encoded UTF-8 string.
        unsafe { std::str::from_utf8_unchecked(&self.encoded) }
    }
}

impl ops::Deref for OutgoingResponse {
    type Target = Response;
    fn deref(&self) -> &Self::Target {
        &self.response
    }
}

impl ops::DerefMut for OutgoingResponse {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.response
    }
}

impl Encode for OutgoingResponse {
    type Buffer = Bytes;
    type Error = std::io::Error;

    fn encode(&self) -> Result<Self::Buffer, std::io::Error> {
        let response = &self.response;
        let buf = BytesMut::new();
        let mut writer = buf.writer();

        write!(
            writer,
            "SIP/2.0 {} {}\r\n",
            response.status_line.code.as_u16(),
            response.status_line.reason.as_str()
        )?;
        write!(writer, "{}", response.headers)?;
        write_body(&mut writer, response.body.as_ref())?;

        Ok(writer.into_inner().freeze())
    }
}

impl Encode for OutgoingRequest {
    type Buffer = Bytes;
    type Error = std::io::Error;

    fn encode(&self) -> Result<Self::Buffer, std::io::Error> {
        let request = &self.request;
        let buf = BytesMut::new();
        let mut writer = buf.writer();

        write!(writer, "{}", request.req_line)?;
        write!(writer, "{}", request.headers)?;
        write_body(&mut writer, request.body.as_ref())?;

        Ok(writer.into_inner().freeze())
    }
}

fn write_body<W: Write>(writer: &mut W, body: Option<&SipBody>) -> std::io::Result<()> {
    const CONTENT_LENGTH: &str = ContentLength::NAME;
    if let Some(body) = body {
        write!(writer, "{CONTENT_LENGTH}: {}\r\n", body.len())?;
        write!(writer, "\r\n")?;
        writer.write_all(body)?;
    } else {
        write!(writer, "{CONTENT_LENGTH}: 0\r\n")?;
        write!(writer, "\r\n")?;
    }
    Ok(())
}

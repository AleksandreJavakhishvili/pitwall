//! The viewer's side of the handshake (tailnet-peers.md §6), shared by
//! probes, pairing and links: connect, `peerHello`, and `peer.auth` with
//! both keys proven.

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream};

use serde::de::DeserializeOwned;

use pitwall_proto::frame::{self, Frame};
use pitwall_proto::peer::{
    transcript, PeerAuth, PeerAuthed, PeerClientHello, PeerHello, PeerServerHello, PeerWelcome, Signer, IDENTITY_CHANGED, PEER_AUTH, PEER_MAX_FRAME,
};
use pitwall_proto::{code, ErrorBody, Range, Request, ServerMsg};

use crate::key::{self, InstanceKey};
use crate::tailnet::Node;
use crate::Ctx;

/// A connection that got a welcome.
pub(crate) struct Dialed {
    pub stream: TcpStream,
    pub reader: BufReader<TcpStream>,
    pub welcome: PeerWelcome,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    nonce: String,
}

fn unreachable(label: &str, e: impl std::fmt::Display) -> ErrorBody {
    ErrorBody::new(code::UNREACHABLE, format!("{label} doesn't answer ({e}); is \"Let my other machines see this Pitwall's agents\" on there?"))
}

/// The next JSON message (other frames skipped), from a peer: frames over
/// `PEER_MAX_FRAME` are an error.
pub(crate) fn read_json<T: DeserializeOwned>(r: &mut impl std::io::Read) -> std::io::Result<Option<T>> {
    loop {
        match frame::read_max(r, PEER_MAX_FRAME)? {
            Some(Frame::Json(body)) => {
                return serde_json::from_slice(&body).map(Some).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
            Some(_) => continue,
            None => return Ok(None),
        }
    }
}

/// Connect to `node` on `port` and say hello.
pub(crate) fn dial(ctx: &Ctx, node: &Node, port: u16) -> Result<Dialed, ErrorBody> {
    let label = node.label();
    let addr = ctx.tailnet.dial_addr(node, port).ok_or_else(|| ErrorBody::new(code::UNREACHABLE, format!("{label} has no Tailscale address")))?;
    let stream = TcpStream::connect_timeout(&addr, ctx.timing.connect).map_err(|e| unreachable(&label, e))?;
    let local = stream.local_addr().map_err(|e| unreachable(&label, e))?;
    ctx.tailnet.dialed(local);
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(ctx.timing.hello)).map_err(|e| unreachable(&label, e))?;
    let nonce = key::nonce();
    let hello = PeerClientHello::PeerHello(PeerHello { protocol: Range::ours(), client: format!("pitwall/{}", ctx.version), nonce: nonce.clone() });
    let mut w = &stream;
    frame::write_json(&mut w, &hello).map_err(|e| unreachable(&label, e))?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| unreachable(&label, e))?);
    let welcome = match read_json::<PeerServerHello>(&mut reader) {
        Ok(Some(PeerServerHello::PeerWelcome(w))) => w,
        Ok(Some(PeerServerHello::Reject(r))) => {
            return Err(ErrorBody::new(code::INCOMPATIBLE, format!("{label} runs an incompatible Pitwall ({}); update one of them", r.daemon)));
        }
        // Closed without a word: not a Pitwall of this user, or it refused.
        Ok(None) => return Err(unreachable(&label, "closed")),
        Err(e) => return Err(unreachable(&label, e)),
    };
    Ok(Dialed { stream, reader, welcome, local, remote: addr, nonce })
}

impl Dialed {
    /// Send a request (ids are the caller's).
    pub fn send(&self, id: u64, method: &str, params: impl serde::Serialize) -> std::io::Result<()> {
        let req = Request { id, method: method.into(), params: serde_json::to_value(params).unwrap_or_default() };
        let mut w = &self.stream;
        w.write_all(&frame::json(&req))?;
        w.flush()
    }

    /// Prove this Pitwall's key and check the peer's: its signature over
    /// our nonce, and — for a paired peer — that it is the key we paired.
    pub fn auth(&mut self, key: &InstanceKey, label: &str, pair: bool, expect: Option<&str>) -> Result<PeerAuthed, ErrorBody> {
        let peer = self.welcome.label.clone();
        let (viewer, listener, protocol) = (self.local.to_string(), self.remote.to_string(), self.welcome.protocol);
        let sig = key.sign(&transcript(Signer::Viewer, &self.welcome.nonce, &viewer, &listener, protocol));
        let auth = PeerAuth { key: key.public_key(), sig, label: label.into(), pair };
        self.send(1, PEER_AUTH, &auth).map_err(|e| unreachable(&peer, e))?;
        let authed = match read_json::<ServerMsg>(&mut self.reader) {
            Ok(Some(ServerMsg::Response(r))) => r.into_result().and_then(|v| {
                serde_json::from_value::<PeerAuthed>(v).map_err(|e| ErrorBody::new(code::OTHER, format!("{peer}: {e}")))
            })?,
            Ok(_) => return Err(unreachable(&peer, "closed")),
            Err(e) => return Err(unreachable(&peer, e)),
        };
        let changed = || ErrorBody::new(IDENTITY_CHANGED, format!("{peer}'s identity changed; pair again"));
        if !InstanceKey::verify(&authed.key, &transcript(Signer::Listener, &self.nonce, &viewer, &listener, protocol), &authed.sig) {
            return Err(changed());
        }
        if expect.is_some_and(|k| k != authed.key) {
            return Err(changed());
        }
        Ok(authed)
    }
}

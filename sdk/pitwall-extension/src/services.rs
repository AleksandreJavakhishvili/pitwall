//! Pitwall's services beyond the tile's own views: HTTP to granted hosts
//! ([`net`], with [`Cx::fetch`](crate::Cx::fetch)), the extension's own
//! SQLite database ([`sql`]), system notifications ([`notify`]), opening
//! a file or a page ([`show`]), the toolbar button ([`toolbar`]) and the
//! extension's own server ([`service`]). `net`, `notify` and `show` need
//! the permission of that name: without it they answer
//! [`HostError::Denied`](crate::HostError::Denied) (or
//! [`net::Error::Denied`]), and the tile carries on.

use crate::bindings::pitwall::tile as wit;

/// HTTP requests to the hosts the user granted. The manifest asks for the
/// `net` permission with its hosts:
///
/// ```json
/// "permissions": { "net": { "reason": "To read your CI runs", "hosts": ["api.github.com"] } }
/// ```
///
/// A host the user sets (their own server) is named by its `url` setting:
/// `"hosts": ["{setting:server}"]` allows the host and port of setting
/// `server`'s value, while it is one (https, or http to this computer),
/// and follows it when the user changes it.
///
/// Requests are made by Pitwall with [`Cx::fetch`](crate::Cx::fetch): it
/// returns at once and calls back with the answer.
///
/// ```ignore
/// let req = net::Request::get("https://api.github.com/repos/o/r/actions/runs").header("Accept", "application/json");
/// cx.fetch(req, |this, answer, cx| {
///     match answer.and_then(|r| r.json::<Runs>()) {
///         Ok(runs) => this.runs = runs,
///         Err(e) => this.error = Some(e.to_string()),
///     }
///     cx.notify();
/// })?;
/// ```
///
/// `https://` only, and plain `http://` only to this computer. Redirects
/// are followed by Pitwall, only to granted hosts. At most 8 requests at
/// once, a body of 8 MiB, an answer of 32 MiB, 30 s.
pub mod net {
    use super::wit::net as w;
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use std::time::Duration;

    /// A request, built with [`Request::get`], [`Request::post`] and the
    /// others.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Request {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        timeout: Option<Duration>,
    }

    impl Request {
        /// A request with this method (`GET`, `POST`, `PUT`, `PATCH`,
        /// `DELETE`, `HEAD`) to this address.
        pub fn new(method: &str, url: impl Into<String>) -> Request {
            Request { method: method.to_string(), url: url.into(), headers: Vec::new(), body: None, timeout: None }
        }

        /// `GET url`.
        pub fn get(url: impl Into<String>) -> Request {
            Request::new("GET", url)
        }

        /// `POST url`.
        pub fn post(url: impl Into<String>) -> Request {
            Request::new("POST", url)
        }

        /// `PUT url`.
        pub fn put(url: impl Into<String>) -> Request {
            Request::new("PUT", url)
        }

        /// `PATCH url`.
        pub fn patch(url: impl Into<String>) -> Request {
            Request::new("PATCH", url)
        }

        /// `DELETE url`.
        pub fn delete(url: impl Into<String>) -> Request {
            Request::new("DELETE", url)
        }

        /// Add a header (`Authorization`, `Accept`…). `Host`,
        /// `Content-Length`, `Cookie` and a few more are Pitwall's.
        pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Request {
            self.headers.push((name.into(), value.into()));
            self
        }

        /// The body.
        pub fn body(mut self, body: impl Into<Vec<u8>>) -> Request {
            self.body = Some(body.into());
            self
        }

        /// `value` as a JSON body, with `Content-Type: application/json`.
        pub fn json<B: Serialize + ?Sized>(self, value: &B) -> Request {
            match serde_json::to_vec(value) {
                Ok(b) => self.header("Content-Type", "application/json").body(b),
                Err(e) => {
                    crate::log::warn(format!("the request's JSON body couldn't be made: {e}"));
                    self
                }
            }
        }

        /// Give up after `time` (at most 120 s; 30 s when not set).
        pub fn timeout(mut self, time: Duration) -> Request {
            self.timeout = Some(time);
            self
        }

        pub(crate) fn wit(self) -> w::Request {
            w::Request {
                method: self.method,
                url: self.url,
                headers: self.headers.into_iter().map(|(name, value)| w::Header { name, value }).collect(),
                body: self.body,
                timeout_ms: self.timeout.map(|t| t.as_millis().min(u32::MAX as u128) as u32),
            }
        }
    }

    /// An answer: any status (check [`Response::ok`]).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Response {
        /// The status code.
        pub status: u16,
        /// The headers, as received (`Set-Cookie` left out).
        pub headers: Vec<(String, String)>,
        /// The body.
        pub body: Vec<u8>,
        /// The address that answered, after redirects.
        pub url: String,
    }

    impl Response {
        /// A 2xx status.
        pub fn ok(&self) -> bool {
            (200..300).contains(&self.status)
        }

        /// Header `name` (any case), the first if several.
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }

        /// The body as text (invalid UTF-8 replaced).
        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }

        /// The body read as JSON. A status that isn't 2xx is an error
        /// ([`Error::Status`]), with the body's first line.
        pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
            if !self.ok() {
                return Err(Error::Status(self.status, self.text().lines().next().unwrap_or("").chars().take(200).collect()));
            }
            serde_json::from_slice(&self.body).map_err(|e| Error::Decode(e.to_string()))
        }
    }

    impl From<w::Response> for Response {
        fn from(r: w::Response) -> Response {
            Response { status: r.status, headers: r.headers.into_iter().map(|h| (h.name, h.value)).collect(), body: r.body, url: r.url }
        }
    }

    /// Why a request got no answer (or not the one wanted).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Error {
        /// Not granted `net`, or a host it may not reach (named here).
        Denied(String),
        /// Over a limit: 8 requests at once, 8 MiB sent, 32 MiB received.
        TooLarge,
        /// The request isn't valid (address, method, header).
        Invalid(String),
        /// No answer in time.
        TimedOut,
        /// It couldn't be sent or the connection failed (offline…).
        Failed(String),
        /// From [`Response::json`]: the status wasn't 2xx.
        Status(u16, String),
        /// From [`Response::json`]: the body isn't the JSON expected.
        Decode(String),
    }

    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Error::Denied(host) => write!(f, "not allowed to reach {host}"),
                Error::TooLarge => write!(f, "over a size limit, or too many requests at once"),
                Error::Invalid(why) => write!(f, "not a valid request: {why}"),
                Error::TimedOut => write!(f, "no answer in time"),
                Error::Failed(why) => write!(f, "couldn't connect: {why}"),
                Error::Status(code, line) if line.is_empty() => write!(f, "the server answered {code}"),
                Error::Status(code, line) => write!(f, "the server answered {code}: {line}"),
                Error::Decode(why) => write!(f, "the answer isn't what was expected: {why}"),
            }
        }
    }

    impl std::error::Error for Error {}

    impl From<w::FetchError> for Error {
        fn from(e: w::FetchError) -> Error {
            match e {
                w::FetchError::Denied(h) => Error::Denied(h),
                w::FetchError::TooLarge => Error::TooLarge,
                w::FetchError::Invalid(s) => Error::Invalid(s),
                w::FetchError::TimedOut => Error::TimedOut,
                w::FetchError::Failed(s) => Error::Failed(s),
            }
        }
    }

    /// A request on its way, from [`Cx::fetch`](crate::Cx::fetch).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RequestId(pub(crate) u32);
}

/// The extension's own SQLite database, run by Pitwall: only its data,
/// kept outside its folder (updates and going back keep it), shared by
/// its tiles. At most 100 MiB unless the manifest asks for more
/// (`"sql": {"reason": …, "mib": 500}`, shown on the permission screen).
///
/// ```ignore
/// sql::migrate(sql::migrations!("migrations/1.sql", "migrations/2.sql"))?;
/// sql::execute("INSERT INTO notes (title) VALUES (?)", ("Standup",))?;
/// for row in sql::query("SELECT id, title FROM notes WHERE done = ?", (false,))?.iter() {
///     let (id, title): (i64, String) = (row.get(0)?, row.get("title")?);
/// }
/// sql::transaction(|| {
///     sql::execute("DELETE FROM notes WHERE done = 1", ())?;
///     sql::execute("UPDATE stats SET cleared = cleared + 1", ())?;
///     Ok(())
/// })?;
/// ```
///
/// A statement runs within the deadline of the call it is in (1 s for a
/// listener); a transaction lasts at most that call. Answers hold at most
/// 10 000 rows and 8 MiB.
pub mod sql {
    use super::wit::sql as w;
    use crate::HostError;

    /// A value in a row or a parameter.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Integer(i64),
        Real(f64),
        Text(String),
        Blob(Vec<u8>),
    }

    impl Value {
        fn wit(&self) -> w::Value {
            match self {
                Value::Null => w::Value::Null,
                Value::Integer(i) => w::Value::Integer(*i),
                Value::Real(r) => w::Value::Real(*r),
                Value::Text(t) => w::Value::Text(t.clone()),
                Value::Blob(b) => w::Value::Blob(b.clone()),
            }
        }

        fn from_wit(v: w::Value) -> Value {
            match v {
                w::Value::Null => Value::Null,
                w::Value::Integer(i) => Value::Integer(i),
                w::Value::Real(r) => Value::Real(r),
                w::Value::Text(t) => Value::Text(t),
                w::Value::Blob(b) => Value::Blob(b),
            }
        }
    }

    macro_rules! into_value {
        ($($t:ty => $v:expr),* $(,)?) => {
            $(impl From<$t> for Value {
                fn from(x: $t) -> Value {
                    let f: fn($t) -> Value = $v;
                    f(x)
                }
            })*
        };
    }

    into_value! {
        i64 => Value::Integer,
        i32 => |x| Value::Integer(x.into()),
        u32 => |x| Value::Integer(x.into()),
        bool => |x| Value::Integer(x.into()),
        f64 => Value::Real,
        String => Value::Text,
        &str => |x| Value::Text(x.to_string()),
        Vec<u8> => Value::Blob,
        &[u8] => |x| Value::Blob(x.to_vec()),
    }

    impl<T: Into<Value>> From<Option<T>> for Value {
        fn from(x: Option<T>) -> Value {
            x.map_or(Value::Null, Into::into)
        }
    }

    /// Parameters for a statement's `?` placeholders: `()`, a tuple
    /// (`(id, "title")`), or a slice or `Vec` of [`Value`]s.
    pub trait Params {
        #[doc(hidden)]
        fn values(self) -> Vec<Value>;
    }

    impl Params for () {
        fn values(self) -> Vec<Value> {
            vec![]
        }
    }

    impl Params for Vec<Value> {
        fn values(self) -> Vec<Value> {
            self
        }
    }

    impl Params for &[Value] {
        fn values(self) -> Vec<Value> {
            self.to_vec()
        }
    }

    macro_rules! tuple_params {
        ($(($($n:ident),+)),* $(,)?) => {
            $(impl<$($n: Into<Value>),+> Params for ($($n,)+) {
                #[allow(non_snake_case)]
                fn values(self) -> Vec<Value> {
                    let ($($n,)+) = self;
                    vec![$($n.into()),+]
                }
            })*
        };
    }

    tuple_params!((A), (A, B), (A, B, C), (A, B, C, D), (A, B, C, D, E), (A, B, C, D, E, F), (A, B, C, D, E, F, G), (A, B, C, D, E, F, G, H));

    /// A value read as a Rust type ([`Row::get`]).
    pub trait FromValue: Sized {
        #[doc(hidden)]
        fn from_value(v: &Value) -> Option<Self>;
    }

    impl FromValue for i64 {
        fn from_value(v: &Value) -> Option<i64> {
            match v {
                Value::Integer(i) => Some(*i),
                _ => None,
            }
        }
    }

    impl FromValue for i32 {
        fn from_value(v: &Value) -> Option<i32> {
            i64::from_value(v).and_then(|i| i32::try_from(i).ok())
        }
    }

    impl FromValue for u32 {
        fn from_value(v: &Value) -> Option<u32> {
            i64::from_value(v).and_then(|i| u32::try_from(i).ok())
        }
    }

    impl FromValue for u64 {
        fn from_value(v: &Value) -> Option<u64> {
            i64::from_value(v).and_then(|i| u64::try_from(i).ok())
        }
    }

    impl FromValue for bool {
        fn from_value(v: &Value) -> Option<bool> {
            i64::from_value(v).map(|i| i != 0)
        }
    }

    impl FromValue for f64 {
        fn from_value(v: &Value) -> Option<f64> {
            match v {
                Value::Real(r) => Some(*r),
                Value::Integer(i) => Some(*i as f64),
                _ => None,
            }
        }
    }

    impl FromValue for String {
        fn from_value(v: &Value) -> Option<String> {
            match v {
                Value::Text(t) => Some(t.clone()),
                _ => None,
            }
        }
    }

    impl FromValue for Vec<u8> {
        fn from_value(v: &Value) -> Option<Vec<u8>> {
            match v {
                Value::Blob(b) => Some(b.clone()),
                Value::Text(t) => Some(t.clone().into_bytes()),
                _ => None,
            }
        }
    }

    impl<T: FromValue> FromValue for Option<T> {
        fn from_value(v: &Value) -> Option<Option<T>> {
            match v {
                Value::Null => Some(None),
                v => T::from_value(v).map(Some),
            }
        }
    }

    impl FromValue for Value {
        fn from_value(v: &Value) -> Option<Value> {
            Some(v.clone())
        }
    }

    /// A column, by position (`0`) or by name (`"title"`).
    pub trait Column {
        #[doc(hidden)]
        fn index(&self, columns: &[String]) -> Option<usize>;
    }

    impl Column for usize {
        fn index(&self, columns: &[String]) -> Option<usize> {
            (*self < columns.len()).then_some(*self)
        }
    }

    impl Column for &str {
        fn index(&self, columns: &[String]) -> Option<usize> {
            columns.iter().position(|c| c == self)
        }
    }

    /// A query's answer.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Rows {
        /// The columns' names.
        pub columns: Vec<String>,
        /// Each row's values.
        pub rows: Vec<Vec<Value>>,
    }

    impl Rows {
        /// The rows, each readable by column.
        pub fn iter(&self) -> impl Iterator<Item = Row<'_>> {
            self.rows.iter().map(|values| Row { columns: &self.columns, values })
        }

        /// How many rows.
        pub fn len(&self) -> usize {
            self.rows.len()
        }

        /// No rows.
        pub fn is_empty(&self) -> bool {
            self.rows.is_empty()
        }
    }

    /// One row.
    #[derive(Debug, Clone, Copy)]
    pub struct Row<'a> {
        columns: &'a [String],
        values: &'a [Value],
    }

    impl Row<'_> {
        /// Column `col` as a `T`. An unknown column, or a value that isn't
        /// a `T` (`NULL` for a non-`Option`), is [`HostError::Decode`].
        pub fn get<T: FromValue>(&self, col: impl Column) -> Result<T, HostError> {
            let i = col.index(self.columns).ok_or_else(|| HostError::Decode("no such column".into()))?;
            let v = &self.values[i];
            T::from_value(v).ok_or_else(|| HostError::Decode(format!("column “{}” is {v:?}", self.columns[i])))
        }
    }

    /// Run one statement; the number of rows it changed.
    pub fn execute(statement: &str, params: impl Params) -> Result<u64, HostError> {
        let p: Vec<w::Value> = params.values().iter().map(Value::wit).collect();
        w::execute(statement, &p).map_err(HostError::from)
    }

    /// Run one statement and return its rows.
    pub fn query(statement: &str, params: impl Params) -> Result<Rows, HostError> {
        let p: Vec<w::Value> = params.values().iter().map(Value::wit).collect();
        let r = w::query(statement, &p).map_err(HostError::from)?;
        Ok(Rows { columns: r.columns, rows: r.rows.into_iter().map(|row| row.into_iter().map(Value::from_wit).collect()).collect() })
    }

    /// Bring the schema up to date: `steps` are the migrations in order
    /// (step 1 first; each one or more statements). The ones the database
    /// hasn't run yet run now, each in a transaction. Returns the version
    /// now. Keep old steps as they are and add new ones at the end.
    pub fn migrate(steps: &[&str]) -> Result<u32, HostError> {
        let steps: Vec<String> = steps.iter().map(|s| s.to_string()).collect();
        w::migrate(&steps).map_err(HostError::from)
    }

    /// Run `f` in a transaction: committed when it returns `Ok`, rolled
    /// back when it returns `Err`.
    pub fn transaction<R>(f: impl FnOnce() -> Result<R, HostError>) -> Result<R, HostError> {
        w::begin().map_err(HostError::from)?;
        match f() {
            Ok(r) => {
                w::commit().map_err(HostError::from)?;
                Ok(r)
            }
            Err(e) => {
                let _ = w::rollback();
                Err(e)
            }
        }
    }

    /// The bytes the database takes now.
    pub fn usage() -> u64 {
        w::usage()
    }

    /// The most it may take, in bytes.
    pub fn quota() -> u64 {
        w::quota()
    }

    /// The migration files named here, read into the component when it
    /// is built (paths from the crate's root, like `include_str!`), in
    /// order, for [`migrate`]: `sql::migrate(sql::migrations!("migrations/1.sql", "migrations/2.sql"))`.
    #[macro_export]
    macro_rules! migrations {
        ($($file:literal),+ $(,)?) => {
            &[$(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $file))),+]
        };
    }

    pub use crate::migrations;
}

/// System notifications. The manifest asks for `notify`; without it,
/// [`HostError::Denied`]. For news inside Pitwall, [`ui::toast`](crate::ui::toast)
/// needs nothing.
pub mod notify {
    use crate::HostError;

    /// Notify the user: shown by the system while Pitwall isn't in front
    /// and as a toast in Pitwall, labelled with the extension's name; a
    /// click opens the tile. A title of at most 80 characters and a line
    /// of at most 200. At most 3 a minute ([`HostError::Busy`] over that);
    /// the same words again within 10 minutes are shown once.
    pub fn send(title: &str, body: &str) -> Result<(), HostError> {
        super::wit::notify::send(title, body).map_err(HostError::from)
    }
}

/// Show the user a file or a page in Pitwall. The manifest asks for
/// `show`, which the user turns on; without it, [`HostError::Denied`].
/// These only show: the tile gets nothing back.
pub mod show {
    use crate::HostError;

    /// Open the file at `path` (absolute) in Pitwall's editor, at `line`
    /// (1 is the first).
    pub fn file(path: &str, line: Option<u32>) -> Result<(), HostError> {
        super::wit::show::file(path, line).map_err(HostError::from)
    }

    /// Open `url` (`https://`, or `http://` to this computer) in a new web
    /// tile.
    pub fn web(url: &str) -> Result<(), HostError> {
        super::wit::show::web(url).map_err(HostError::from)
    }

    /// Show the file at `path` (absolute) in Pitwall's file viewer,
    /// scrolled to `line` (1 is the first), as an agent's `show_files`
    /// shows files: with its folder's tree (its git repository's) and git
    /// status, in the tile's own viewer window, which opens behind the
    /// front window without taking the keyboard; later calls add tabs to
    /// it. [`file`] opens Pitwall's editor instead.
    pub fn view(path: &str, line: Option<u32>) -> Result<(), HostError> {
        super::wit::show::view(path, line).map_err(HostError::from)
    }
}

/// The extension's button in Pitwall's top bar, when the manifest
/// declares one (`contributes.toolbar`). Pitwall draws it; the tile sets
/// its badge and tooltip. Without a toolbar item these do nothing.
///
/// A `popover` action opens a view with id [`toolbar::POPOVER_VIEW`]: the
/// tile renders it like any other view (keep it small: 200 × 100 to
/// 420 × 560). A click with a `tile` or `popover` action also arrives as
/// [`Event::ToolbarClicked`](crate::Event::ToolbarClicked).
pub mod toolbar {
    use super::wit::toolbar as w;

    /// The view id of the toolbar popover.
    pub const POPOVER_VIEW: &str = "toolbar:popover";

    /// A badge.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Badge {
        /// No badge.
        None,
        /// A dot.
        Dot,
        /// A number (up to 99, then "99+").
        Count(u32),
    }

    /// A badge's colour.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum Tone {
        /// The accent colour.
        #[default]
        Normal,
        /// Amber, like "needs you".
        Attention,
    }

    fn wit(badge: Badge, tone: Tone) -> (w::Badge, w::BadgeTone) {
        let b = match badge {
            Badge::None => w::Badge::None,
            Badge::Dot => w::Badge::Dot,
            Badge::Count(n) => w::Badge::Count(n),
        };
        let t = match tone {
            Tone::Normal => w::BadgeTone::Normal,
            Tone::Attention => w::BadgeTone::Attention,
        };
        (b, t)
    }

    /// The badge, in every window. At most 4 changes a second are drawn;
    /// newer ones replace the one waiting.
    pub fn set_badge(badge: Badge, tone: Tone) {
        let (b, t) = wit(badge, tone);
        w::set_badge(b, t);
    }

    /// A project's badge, for an item with `"scope": "project"` (needs
    /// `projects.read`). `project` is the id
    /// [`Event::ToolbarClicked`](crate::Event::ToolbarClicked) gives.
    pub fn set_project_badge(project: &str, badge: Badge, tone: Tone) {
        let (b, t) = wit(badge, tone);
        w::set_project_badge(project, b, t);
    }

    /// The tooltip ("3 unread"), at most 80 characters.
    pub fn set_tooltip(text: &str) {
        w::set_tooltip(text);
    }

    /// Show or hide the button; never over the user's Hide. At most one
    /// change in 5 seconds.
    pub fn set_shown(shown: bool) {
        w::set_shown(shown);
    }

    /// Close the popover, if open.
    pub fn close_popover() {
        w::close_popover();
    }
}

/// The extension's own server, when the manifest declares a `service`:
/// its state, and its address, which the tile may reach with
/// [`Cx::fetch`](crate::Cx::fetch) without the `net` permission.
///
/// ```ignore
/// if let Some(url) = service::url("/api/items") {
///     cx.fetch(net::Request::get(url), |this, answer, cx| { /* … */ })?;
/// }
/// ```
pub mod service {
    use super::wit::service as w;

    /// How the server stands.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        /// The manifest declares no service.
        None,
        /// Not started: the user hasn't confirmed its commands yet.
        Off,
        /// Its program isn't installed (or is too old).
        NeedsInstall,
        /// Started, not answering yet.
        Starting,
        /// Running and answering.
        Up,
        /// Stopped by the user.
        Stopped,
        /// It stopped by itself.
        Failed,
    }

    /// The server's state.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Info {
        pub state: State,
        /// `http://127.0.0.1:8790`, when it declares `ready.listen`.
        pub address: Option<String>,
        /// The version its check found.
        pub version: Option<String>,
        /// Why it is off, failed or needs installing.
        pub why: Option<String>,
    }

    impl From<w::ServiceInfo> for Info {
        fn from(i: w::ServiceInfo) -> Info {
            let state = match i.state {
                w::State::None => State::None,
                w::State::Off => State::Off,
                w::State::NeedsInstall => State::NeedsInstall,
                w::State::Starting => State::Starting,
                w::State::Up => State::Up,
                w::State::Stopped => State::Stopped,
                w::State::Failed => State::Failed,
            };
            Info { state, address: i.address, version: i.version, why: i.why }
        }
    }

    /// The server's state now ([`Event::ServiceChanged`](crate::Event::ServiceChanged)
    /// says when it changes).
    pub fn info() -> Info {
        w::info().into()
    }

    /// The address of `path` on the server (`/api/items` →
    /// `http://127.0.0.1:8790/api/items`); `None` while it has no address.
    pub fn url(path: &str) -> Option<String> {
        let base = info().address?;
        Some(format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/')))
    }

    /// Start the server again (as the card's Restart).
    pub fn restart() {
        w::restart();
    }

    /// Show the extension's card in Settings, where the user installs and
    /// starts the server (for a "Set up" button).
    pub fn open_card() {
        w::open_card();
    }
}

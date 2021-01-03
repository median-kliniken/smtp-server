use std::path::PathBuf;

use kannader_config::{reply, server};
use smtp_message::{Email, Hostname, Reply};

struct Config;

impl kannader_config::Config for Config {
    fn setup(_path: PathBuf) -> Config {
        Config
    }
}

kannader_config::implement_guest!(Config);

struct ServerConfig;

impl kannader_config::ServerConfig for ServerConfig {
    type Cfg = Config;

    fn welcome_banner_reply(_cfg: &Config, _conn_meta: &mut server::ConnMeta) -> Reply {
        reply::welcome_banner("localhost", "Service ready")
    }

    fn filter_hello(
        cfg: &Config,
        is_ehlo: bool,
        hostname: Hostname,
        conn_meta: &mut server::ConnMeta,
    ) -> server::SerializableDecision<server::HelloInfo> {
        let mut cm = conn_meta.clone();
        cm.hello = Some(server::HelloInfo {
            is_ehlo,
            hostname: hostname.clone(),
        });
        server::SerializableDecision::Accept {
            reply: reply::okay_hello(is_ehlo, "localhost", "", Self::can_do_tls(cfg, cm)).convert(),
            res: server::HelloInfo { is_ehlo, hostname },
        }
    }

    fn new_mail(_cfg: &Config, _conn_meta: &mut server::ConnMeta) -> Vec<u8> {
        Vec::new()
    }

    fn filter_from(
        _cfg: &Config,
        from: Option<Email>,
        _meta: &mut server::MailMeta,
        _conn_meta: &mut server::ConnMeta,
    ) -> server::SerializableDecision<Option<Email>> {
        server::SerializableDecision::Accept {
            reply: reply::okay_from().convert(),
            res: from,
        }
    }

    fn filter_to(
        _cfg: &Config,
        to: Email,
        _meta: &mut server::MailMeta,
        _conn_meta: &mut server::ConnMeta,
    ) -> server::SerializableDecision<Email> {
        // TODO TODO THIS IS BAD
        server::SerializableDecision::Accept {
            reply: reply::okay_to().convert(),
            res: to,
        }
    }
}

kannader_config::server_config_implement_guest!(ServerConfig);
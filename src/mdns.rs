use mdns_sd::{ServiceDaemon, ServiceInfo, TxtProperty};

pub fn register_service(hostname: &str, port: u16) -> Option<ServiceDaemon> {
    let daemon = ServiceDaemon::new().ok()?;

    let service_type = "_opencode._tcp.local.";
    let instance_name = format!("{}", hostname);
    let host = format!("{}.local.", hostname);

    let props = vec![
        TxtProperty::from(("version".to_string(), env!("CARGO_PKG_VERSION").to_string())),
        TxtProperty::from(("protocol".to_string(), "http".to_string())),
    ];

    let service_info = ServiceInfo::new(
        service_type,
        &instance_name,
        &host,
        "",
        port,
        props,
    )
    .ok()?;

    daemon.register(service_info).ok()?;
    tracing::info!("mDNS: registered {} on port {}", hostname, port);
    Some(daemon)
}

#[allow(dead_code)]
pub fn unregister_service(daemon: &ServiceDaemon, fullname: &str) {
    let _ = daemon.unregister(fullname);
}

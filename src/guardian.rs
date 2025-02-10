use crate::{
    receiver::receiver,
    sender::{self, sender},
    socket::Sockets,
    updater::updater,
    Discoverer,
};
use acto::{AcTokioRuntime, ActoCell, ActoInput};
use hickory_proto::rr::Name;
use std::{mem::replace, net::IpAddr};

pub enum Input {
    RemoveAll,
    RemovePort(u16),
    RemoveAddr(IpAddr),
    AddAddr(u16, Vec<IpAddr>),
    SetTxt(String, Option<String>),
    RemoveTxt(String),
}

pub async fn guardian(
    mut ctx: ActoCell<Input, AcTokioRuntime, anyhow::Result<()>>,
    mut discoverer: Discoverer,
    sockets: Sockets,
    service_name: Name,
) {
    let callback = replace(&mut discoverer.callback, Box::new(|_, _| {}));
    let tau = discoverer.tau;
    let phi = discoverer.phi;
    let upd_ref = ctx.supervise(
        ctx.spawn("updater", move |ctx| updater(ctx, tau, phi, callback))
            .map_handle(Ok),
    );

    let sockets2 = sockets.clone();
    let sn = service_name.clone();
    let snd_ref = ctx.supervise(
        ctx.spawn("sender", move |ctx| {
            sender(ctx, sockets, upd_ref, discoverer, sn)
        })
        .map_handle(Ok),
    );

    if let Some(v4) = sockets2.v4() {
        let service_name = service_name.clone();
        let snd_ref = snd_ref.clone();
        ctx.spawn_supervised("receiver_v4", move |ctx| {
            receiver(ctx, service_name, v4, snd_ref)
        });
    }

    if let Some(v6) = sockets2.v6() {
        let snd_ref = snd_ref.clone();
        ctx.spawn_supervised("receiver_v6", move |ctx| {
            receiver(ctx, service_name, v6, snd_ref)
        });
    }

    // only stop when a supervised actor stops
    loop {
        let msg = ctx.recv().await;
        match msg {
            ActoInput::NoMoreSenders => {}
            ActoInput::Supervision { id, name, result } => {
                match result {
                    Ok(Ok(_)) => tracing::warn!("actor {:?} ({}) stopped", id, name),
                    Ok(Err(e)) => {
                        tracing::warn!("actor {:?} ({}) failed: {}", id, name, e)
                    }
                    Err(e) => {
                        tracing::warn!("actor {:?} ({}) aborted: {}", id, name, e);
                    }
                }
                break;
            }
            ActoInput::Message(msg) => {
                snd_ref.send(sender::MdnsMsg::Update(msg));
            }
        }
    }
}

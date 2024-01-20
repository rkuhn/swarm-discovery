use crate::{
    socket::{Mode, Sockets},
    Discoverer, Peer,
};
use acto::{AcTokioRuntime, ActoCell, ActoInput, ActoRef};
use hickory_proto::{
    op::{Message, MessageType, Query},
    rr::{rdata, DNSClass, Name, RData, Record, RecordType},
};
use rand::{thread_rng, Rng};
use std::{collections::BTreeMap, net::IpAddr, str::FromStr, time::Duration};

pub enum MdnsMsg {
    QueryV4,
    QueryV6,
    Response(BTreeMap<String, Peer>),
    Timeout(usize),
}

pub async fn sender(
    mut ctx: ActoCell<MdnsMsg, AcTokioRuntime>,
    sockets: Sockets,
    updater: ActoRef<BTreeMap<String, Peer>>,
    discoverer: Discoverer,
    service_name: Name,
) {
    let tau = discoverer.tau;
    let phi = discoverer.phi;

    let query = make_query(&service_name);
    let response = make_response(&discoverer, &service_name);

    let mut timeout_count = 0;

    loop {
        let me = ctx.me();
        let timeout = tokio::spawn(async move {
            let millionth = thread_rng().gen_range(0..1_000_000);
            tokio::time::sleep(tau + tau / 1_000_000 * millionth).await;
            me.send(MdnsMsg::Timeout(timeout_count));
        });

        let mode = loop {
            if let ActoInput::Message(msg) = ctx.recv().await {
                match msg {
                    MdnsMsg::QueryV4 => {
                        timeout.abort();
                        break Mode::V4;
                    }
                    MdnsMsg::QueryV6 => {
                        timeout.abort();
                        break Mode::V6;
                    }
                    MdnsMsg::Response(resp) => {
                        updater.send(resp);
                    }
                    MdnsMsg::Timeout(count) if count == timeout_count => {
                        sockets.send_msg(&query, Mode::Any).await;
                        break Mode::Any;
                    }
                    MdnsMsg::Timeout(_) => {}
                }
            } else {
                return;
            }
        };

        timeout_count += 1;

        let me = ctx.me();
        let timeout = tokio::spawn(async move {
            let millionth = thread_rng().gen_range(0..1_000_000);
            tokio::time::sleep(Duration::from_millis(100) / 1_000_000 * millionth).await;
            me.send(MdnsMsg::Timeout(timeout_count));
        });

        let mut response_count = 0;
        loop {
            if let ActoInput::Message(msg) = ctx.recv().await {
                match msg {
                    MdnsMsg::Response(resp) => {
                        response_count += resp.len();
                        updater.send(resp);
                        if response_count > (tau.as_secs_f32() * phi) as usize {
                            timeout.abort();
                            break;
                        }
                    }
                    MdnsMsg::Timeout(count) if count == timeout_count => {
                        if let Some(response) = &response {
                            sockets.send_msg(response, mode).await;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }

        timeout_count += 1;
    }
}

fn make_query(service_name: &Name) -> Message {
    let mut msg = Message::new();
    msg.set_message_type(MessageType::Query);
    let mut query = Query::new();
    query.set_query_class(DNSClass::IN);
    query.set_query_type(RecordType::PTR);
    query.set_name(service_name.clone());
    msg.add_query(query);
    msg
}

fn make_response(discoverer: &Discoverer, service_name: &Name) -> Option<Message> {
    if let Some(peer) = discoverer.peers.get(&discoverer.peer_id) {
        let mut msg = Message::new();
        msg.set_message_type(MessageType::Response);
        msg.set_authoritative(true);

        let target = Name::from_str(&format!("{}.local.", discoverer.peer_id))
            .expect("PeerId was checked in spawn()");
        let my_srv_name = Name::from_str(&discoverer.peer_id)
            .expect("PeerId was checked in spawn()")
            .append_domain(service_name)
            .expect("was checked in spawn()");
        msg.add_answer(Record::from_rdata(
            my_srv_name,
            0,
            RData::SRV(rdata::SRV::new(0, 0, peer.port, target.clone())),
        ));
        for addr in &peer.addrs {
            match addr {
                IpAddr::V4(addr) => {
                    msg.add_additional(Record::from_rdata(
                        target.clone(),
                        0,
                        RData::A(rdata::A::from(*addr)),
                    ));
                }
                IpAddr::V6(addr) => {
                    msg.add_additional(Record::from_rdata(
                        target.clone(),
                        0,
                        RData::AAAA(rdata::AAAA::from(*addr)),
                    ));
                }
            }
        }
        Some(msg)
    } else {
        tracing::info!("no addresses for peer, not announcing");
        None
    }
}

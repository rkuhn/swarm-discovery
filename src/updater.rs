use crate::Peer;
use acto::{ActoCell, ActoInput, ActoRuntime};
use std::collections::BTreeMap;

pub async fn updater(
    mut ctx: ActoCell<BTreeMap<String, Peer>, impl ActoRuntime>,
    mut callback: Box<dyn FnMut(&str, &Peer) + Send + Sync>,
) {
    let mut peers = BTreeMap::new();
    while let ActoInput::Message(msg) = ctx.recv().await {
        for (id, peer) in msg {
            if let Some(old) = peers.get(&id) {
                if old != &peer {
                    callback(&id, &peer);
                    peers.insert(id, peer);
                }
            } else {
                callback(&id, &peer);
                peers.insert(id, peer);
            }
        }
    }
}

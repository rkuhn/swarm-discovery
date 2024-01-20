This library offers a lightweight discovery service based on mDNS.
It is geared towards facilitating new peers discovering a swarm in the local network to allow joining to be implemented without needing a preconfigured contact point.
The difference to using plain mDNS is that this library carefully chooses when to send queries or responses such that regardless of swarm size the used network bandwidth does not grow out of bounds.
At the same time, it maintains a steady flow of packets which can serve as an indicator of liveness and connectedness.

## Algorithm

Configuration parameters: τ (discovery time target), φ (response frequency target)

Each node sends mDNS queries according to the following algorithm:

1. set timeout to a duration pulled at random from the interval [τ, 2τ].
2. upon reception of another node’s mDNS query go back to 1.
3. upon timeout, send mDNS query and go back to 1.

Each node responds to a mDNS query according to the following algorithm:

1. at reception of mDNS query, set timeout to a duration pulled at random from the interval [0, 100ms].
2. set counter to zero.
3. upon reception of another node’s mDNS response to this query, increment the counter.
4. when counter is greater than τ × φ end procedure.
5. upon timeout send response.

## mDNS usage

- configurable service name NAME
- queries sent for PTR records of the form `_NAME._tcp.local.`
- responses give SRV records of the form `PEER_ID._NAME._tcp.local.` -> `PEER_ID.local.` (and associated A/AAAA records)

use crate::device::{self, FutureDevice};
use crate::socket_alloctor::SocketAlloctor;
use crate::BufferSize;
use smoltcp::{
    iface::EthernetInterface,
    time::{Duration, Instant},
};
use std::{future::Future, sync::Arc};
use tokio::{pin, select, sync::Notify, time::sleep};

pub struct Reactor {
    socket_alloctor: Arc<SocketAlloctor>,
    notify: Arc<Notify>,
}

async fn run<S: device::Interface + 'static>(
    mut interf: EthernetInterface<'static, FutureDevice<S>>,
    sockets: Arc<SocketAlloctor>,
    notify: Arc<Notify>,
    stopper: Arc<Notify>,
) {
    let default_timeout = Duration::from_secs(60);
    let timer = sleep(default_timeout.into());
    pin!(timer);

    loop {
        let start = Instant::now();
        let deadline = {
            interf
                .poll_delay(&*sockets.lock(), start)
                .unwrap_or(default_timeout)
        };
        let device = interf.device_mut();
        device.send_queue().await.expect("Failed to send queue");

        if device.need_wait() {
            timer
                .as_mut()
                .reset(tokio::time::Instant::now() + deadline.into());
            select! {
                _ = &mut timer => {},
                _ = device.wait() => {}
                _ = notify.notified() => {}
                _ = stopper.notified() => break,
            };
        }

        let mut set = sockets.lock();
        let end = Instant::now();
        match interf.poll(&mut set, end) {
            Ok(true) => (),
            // readiness not changed
            Ok(false) | Err(smoltcp::Error::Dropped) => continue,
            Err(_e) => {
                continue;
            }
        };
    }
}

impl Reactor {
    pub fn new<S: device::Interface + 'static>(
        interf: EthernetInterface<'static, FutureDevice<S>>,
        buffer_size: BufferSize,
        stopper: Arc<Notify>,
    ) -> (Self, impl Future<Output = ()> + Send) {
        let socket_alloctor = Arc::new(SocketAlloctor::new(buffer_size));
        let notify = Arc::new(Notify::new());
        let fut = run(interf, socket_alloctor.clone(), notify.clone(), stopper);

        (
            Reactor {
                socket_alloctor,
                notify,
            },
            fut,
        )
    }
    pub fn socket_alloctor(&self) -> &Arc<SocketAlloctor> {
        &self.socket_alloctor
    }
    pub fn notify(&self) {
        self.notify.clone().notify_waiters();
    }
}

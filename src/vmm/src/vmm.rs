use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use crate::{
    config::Config,
    constants,
    device::SharedDeviceManager,
    memory::Memory,
    vcpu::ExitHandler,
    vm::{Vm, VmConfig},
};
use event_manager::{EventManager, EventOps, EventSet, Events, MutEventSubscriber, SubscriberOps};
use kvm_ioctls::Kvm;
use util::result::{bail, Result};
use vmm_sys_util::eventfd::EventFd;

pub struct Vmm {
    config: Config,
    memory: Memory,
    device_manager: SharedDeviceManager,
    event_manager: EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    vm: Vm<SharedExitEventHandler>,
}

impl Vmm {
    pub fn new(config: Config) -> Result<Self> {
        let kvm = Kvm::new()?;
        Vmm::check_kvm_caps(&kvm)?;

        let memory = Memory::new(config.memory.clone())?;
        let device_manager = SharedDeviceManager::new();

        let vm_config = VmConfig::new(&kvm, config.vcpu.num, constants::MAX_IRQ)?;
        let exit_handler = SharedExitEventHandler::new()?;

        let vm = Vm::new(
            &kvm,
            &memory,
            vm_config,
            exit_handler.clone(),
            device_manager.clone(),
        )?;

        let mut event_manager = EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new()?;
        event_manager.add_subscriber(exit_handler.0.clone());

        Ok(Vmm {
            config,
            memory,
            device_manager,
            event_manager,
            vm,
        })
    }

    fn check_kvm_caps(kvm: &Kvm) -> Result<()> {
        let required_caps = vec![
            kvm_ioctls::Cap::Irqchip,
            kvm_ioctls::Cap::Ioeventfd,
            kvm_ioctls::Cap::Irqfd,
            kvm_ioctls::Cap::UserMemory,
        ];

        for cap in required_caps {
            if !kvm.check_extension(cap) {
                bail!("required KVM cap not supported: {:?}", cap);
            }
        }

        Ok(())
    }
}

struct ExitEventHandler {
    exit_event: EventFd,
    keep_running: AtomicBool,
}

#[derive(Clone)]
struct SharedExitEventHandler(Arc<Mutex<ExitEventHandler>>);

impl SharedExitEventHandler {
    pub fn new() -> Result<Self> {
        let exit_event = EventFd::new(libc::EFD_NONBLOCK)?;
        let keep_running = AtomicBool::new(true);

        let exit_handler = ExitEventHandler {
            exit_event,
            keep_running,
        };

        Ok(SharedExitEventHandler(Arc::new(Mutex::new(exit_handler))))
    }
}

impl ExitHandler for SharedExitEventHandler {
    fn kick(&self) -> Result<()> {
        Ok(self.0.lock().unwrap().exit_event.write(1)?)
    }
}

impl MutEventSubscriber for ExitEventHandler {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set().contains(EventSet::IN) {
            self.keep_running.store(false, Ordering::Release);
        }
        if events.event_set().contains(EventSet::ERROR) {
            // We cannot do much about the error (besides log it).
            // TODO: log this error once we have a logger set up.
            let _ = ops.remove(Events::new(&self.exit_event, EventSet::IN));
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::new(&self.exit_event, EventSet::IN))
            .expect("Cannot initialize exit handler.");
    }
}

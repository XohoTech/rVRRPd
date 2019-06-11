//! finite-state machine module
//! This module includes the finite-state machine (FSM).
use super::*;

// channels and threads
use std::sync::{Arc, Mutex};

// threads
use std::thread;

// debugging
use crate::debug::{Verbose};

/// Virtual Router Parameters Structure
#[derive(Debug)]
pub struct Parameters {
    vrid: u8,                                              // Virtual Router Identifier (1-255)
    interface: String,           // Interface where the virtual router is running
    ifindex: i32,                // Interface ifindex
    prio: u8,                    // Priority (0-255)
    vip: [u8; 4],                // Virtual IP (not in RFC parameters list)
    ipaddrs: Vec<[u8; 4]>, // One or more local IPv4 Addresse(s) associated with the virtual router
    ipmasks: Vec<[u8; 4]>, // IPv4 Netmask(s) of above IP addresses
    adverint: u8,          // Advertisement interval
    skew_time: f32,        // Time to skew Master_Down interval (second)
    master_down: f32,      // Time interval for Backup to declare Master Down
    preempt_mode: bool, // Control whether a higher-priority Backup router can preempt a lower-priority Master
    rfc3768: bool,      // RFC2338 compatibility flag
    auth_type: u8,      // Authentication type being used
    auth_data: [u8; 8], // Autentication data (type specific)
    auth_secret: Option<String>, // Authentication secret for type-1 RFC3768 authentication
    notification: Option<Arc<Mutex<mpsc::Sender<Event>>>>, // Notification channel
    protocols: Arc<Mutex<Protocols>>, // Internal protocols information
    ifmac: [u8; 6],     // Interface Ethernet MAC address
}

/// Parameters Type Implementation
impl Parameters {
    // new() method
    pub fn new(
        vrid: u8,
        interface: String,
        ifindex: i32,
        prio: u8,
        vip: [u8; 4],
        ipaddrs: Vec<[u8; 4]>,
        ipmasks: Vec<[u8; 4]>,
        adverint: u8,
        skew_time: f32,
        master_down: f32,
        preempt_mode: bool,
        rfc3768: bool,
        auth_type: u8,
        auth_data: [u8; 8],
        auth_secret: Option<String>,
        protocols: Arc<Mutex<Protocols>>,
    ) -> Parameters {
        Parameters {
            vrid,
            interface,
            ifindex,
            prio,
            vip,
            ipaddrs,
            ipmasks,
            adverint,
            skew_time,
            master_down,
            preempt_mode,
            rfc3768,
            auth_type,
            auth_data,
            auth_secret,
            notification: Option::None,
            protocols,
            ifmac: [0, 0, 0, 0, 0, 0],
        }
    }
    // vrid() getter
    pub fn vrid(&self) -> u8 {
        self.vrid
    }
    // interface() getter
    pub fn interface(&self) -> &String {
        &self.interface
    }
    // ifindex() getter
    pub fn ifindex(&self) -> i32 {
        self.ifindex
    }
    // prio() getter
    pub fn prio(&self) -> u8 {
        self.prio
    }
    // vip() getter
    pub fn vip(&self) -> [u8; 4] {
        self.vip
    }
    // ipaddrs() getter
    pub fn ipaddrs(&self) -> &Vec<[u8; 4]> {
        &self.ipaddrs
    }
    // adverint() getter
    pub fn adverint(&self) -> u8 {
        self.adverint
    }
    // rfc3768() getter
    pub fn rfc3768(&self) -> bool {
        self.rfc3768
    }
    // authtype() getter
    pub fn authtype(&self) -> u8 {
        self.auth_type
    }
    // authsecret() getter
    pub fn authsecret(&self) -> &Option<String> {
        &self.auth_secret
    }
    // addrcount() method
    pub fn addrcount(&self) -> u8 {
        // calculate the number of addresses (or arrays) in ipaddrs vector
        let num = *&self.ipaddrs.len() as u8;
        // if rfc3768 compatibility flag is false, add one to account for the VIP
        if !self.rfc3768 {
            num + 1
        } else {
            num
        }
    }
    // primary_ip() method
    pub fn primary_ip(&self) -> [u8; 4] {
        // return the first array in vector
        self.ipaddrs[0]
    }
    // notification method
    pub fn notification(&self) -> &Option<Arc<Mutex<mpsc::Sender<Event>>>> {
        &self.notification
    }
}

/// Internal Protocol States "Enumerator"
#[derive(Debug)]
pub enum States {
    Down,   // Special down state
    Init,   // Initialize
    Backup, // Virtual Router is Backup
    Master, // Virtual Router is Master
}

/// Timers Structure
#[derive(Debug)]
pub struct Timers {
    master_down: f32, // Timer that fires when ADVERTISEMENT has not been heared for 'master_down'
    advert: u8,       // Timer that fires sending of ADVERTISEMENT every 'advertint' interval
}

// Timers Type Implementation
impl Timers {
    // new() method
    pub fn new(master_down: f32, advert: u8) -> Timers {
        Timers {
            master_down,
            advert,
        }
    }
    // master_down() getter
    pub fn master_down(&self) -> f32 {
        self.master_down
    }
    // advert() getter
    pub fn advert(&self) -> u8 {
        // make sure the advertisement timer is higher than zero
        //assert!(self.advert > 0);
        self.advert
    }
}

/// Flags Structure
#[derive(Debug)]
pub struct Flags {
    down: u8,
}

// Flags Type Implementation
impl Flags {
    // new() method
    pub fn new(down: u8) -> Flags {
        Flags { down }
    }
    // get_down_flag() method
    pub fn get_down_flag(&self) -> u8 {
        self.down
    }
    // set_down_flag() method
    pub fn set_down_flag(&mut self) {
        self.down = 0x1;
    }
    // clear_down_flag() method
    pub fn clear_down_flag(&mut self) {
        self.down = 0x0;
    }
}

/// Event Enumerator
#[derive(Debug)]
pub enum Event {
    Startup,
    Shutdown, // Internal Shutdown Event
    Terminate,
    MasterDown,          // internal master down notification
    MasterDownExpiry,    // internal master_down timer expiry notification
    Advert([u8; 4], u8), // got ADVERTISEMENT message (carrying priority)
    GenAdvert,           // generate an ADVERTISEMENT message
}

// fsm_run() function
/// run the finite-state machine (FSM)
pub fn fsm_run(
    id: usize,
    tx: &Arc<Mutex<mpsc::Sender<Event>>>,
    rx: &Arc<Mutex<mpsc::Receiver<Event>>>,
    vr: &Arc<RwLock<VirtualRouter>>,
    sockfd: i32,
    debug: &Verbose,
) {
    // print debugging information
    print_debug(
        debug,
        DEBUG_LEVEL_HIGH,
        format!(
            "debug(fsm): registering notification sender channel for thread {}",
            id
        ),
    );

    // register notification sender channel
    register_tx(&vr, &tx, id, &debug);

    // start thread loop
    loop {
        // print debugging information
        print_debug(
            debug,
            DEBUG_LEVEL_EXTENSIVE,
            format!(
                "debug(worker): worker thread {} acquiring lock on rx channel",
                id
            ),
        );
        // acquire lock on receive channel
        let event = rx.lock().unwrap();

        // print debugging information
        print_debug(
            debug,
            DEBUG_LEVEL_EXTENSIVE,
            format!("debug(worker): worker thread {} waiting for events", id),
        );
        // listen for events
        let event = event.recv().unwrap();
        // print debugging information
        print_debug(
            debug,
            DEBUG_LEVEL_EXTENSIVE,
            format!(
                "debug(worker): worker thread {} got new {:?} event",
                id, event
            ),
        );

        // handle terminate event first and foremost
        match event {
            // got Termination event
            Event::Terminate => {
                // print debugging information
                print_debug(
                    debug,
                    DEBUG_LEVEL_HIGH,
                    format!("Worker thread {} exited", id),
                );
                // break current loop
                break;
            }
            _ => {}
        }

        // clone tx channel and vr for timer thread(s)
        let timer_tx = Arc::clone(&tx);
        let timer_vr = Arc::clone(&vr);

        // print debugging information
        print_debug(
            debug,
            DEBUG_LEVEL_EXTENSIVE,
            format!("debug(worker): worker thread {} acquiring write lock", id),
        );
        // acquire write lock on thread's virtual router
        let mut vr = vr.write().unwrap();
        // print debugging information
        print_debug(
            debug,
            DEBUG_LEVEL_EXTENSIVE,
            format!("debug(worker): worker thread {} write lock acquired", id),
        );

        // evaluate virtual router's current state
        vr.states = match &vr.states {
            States::Down => {
                continue;
            }
            States::Init => {
                match event {
                    // event: if a Startup event is received
                    Event::Startup => {
                        // print debugging information
                        print_debug(
                            debug,
                            DEBUG_LEVEL_EXTENSIVE,
                            format!("debug(fsm): got Startup event on thread {}", id),
                        );

                        // print information
                        println!(
                            "Starting VRRP Virtual Router ({}.{}.{}.{}) for group {}, on interface {} (thread: {})",
                            vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                            vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface,
                            id
                        );

                        // print debugging information
                        print_debug(
                            debug,
                            DEBUG_LEVEL_EXTENSIVE,
                            format!(
                                "debug(fsm): starting timer thread from worker thread {}",
                                id
                            ),
                        );
                        // starting timer thread(s)
                        // and clone debug structure of type Verbose
                        let d = debug.clone();
                        let _timer_thread = thread::spawn(move || {
                            timers::start_timers(timer_tx, timer_vr, &d);
                        });

                        // if the virtual router is the owner of the virtual ip address
                        // OR the priority has been configured at 255
                        if vr.is_owner_vip(&vr.parameters.vip) || vr.parameters.prio == 255 {
                            // force the priority to 255
                            vr.parameters.prio = 255;
                            // send an ADVERTISEMENT message
                            // and panic on error
                            packets::send_advertisement(sockfd, &vr, &debug).unwrap();
                            // broadcast a gratuitious ARP request
                            let arp_sockfd = arp::open_raw_socket_arp().unwrap();
                            arp::broadcast_gratuitious_arp(arp_sockfd, &vr).unwrap();
                            // set advertisement interval
                            vr.timers.advert = vr.parameters.adverint;
                            // print debugging information
                            print_debug(
                                &debug,
                                DEBUG_LEVEL_EXTENSIVE,
                                format!(
                                    "debug(fsm): the advertisement interval is now {}s",
                                    vr.timers.advert
                                ),
                            );
                            // print information
                            println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Init to Master",
                                vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                                vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                            );
                            // transition to Master state
                            fsm::States::Master
                        } else {
                            // set master_down timer
                            vr.timers.master_down = vr.parameters.master_down;
                            // print information
                            println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Init to Backup",
                                vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                                vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                            );
                            // transition to Backup state
                            States::Backup
                        }
                    }
                    // event: if Shutdown event is received
                    Event::Shutdown => {
                        // print information
                        println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Init to Down",
                                vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                                vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                        );
                        // transition to Down state
                        States::Down
                    }
                    _ => {
                        // print debugging information
                        print_debug(
                            debug,
                            DEBUG_LEVEL_EXTENSIVE,
                            format!("debug(fsm): unexpected event catched in Init state"),
                        );
                        continue;
                    }
                }
            }
            States::Backup => {
                match event {
                    // event: If ADVERTISEMENT message is received
                    Event::Advert(_ipsrc, prio) => {
                        // if the priority is zero then set the master_down timer to skew_time
                        if prio == 0 {
                            // set master_down interval to skew_time (necessary?)
                            vr.timers.master_down = vr.parameters.skew_time;
                        } else {
                            // if priority is greater than or equal to the local priority OR preempt is false
                            if vr.parameters.preempt_mode == false || prio >= vr.parameters.prio {
                                // reset master_down interval (necessary?)
                                vr.timers.master_down = vr.parameters.master_down;
                                // clear down flag (signal master is alive)
                                vr.flags.clear_down_flag();
                                // print debugging information
                                print_debug(
                                    debug,
                                    DEBUG_LEVEL_HIGH,
                                    format!("debug(fsm): down flag cleared in Backup state"),
                                );
                            }
                        }
                        continue;
                    }
                    // event: master_down timer has expired
                    Event::MasterDownExpiry => {
                        // set down flag
                        vr.flags.set_down_flag();
                        // print debugging information
                        print_debug(
                            debug,
                            DEBUG_LEVEL_HIGH,
                            format!("debug(timer): down flag set for worker thread {}", id),
                        );
                        continue;
                    }
                    // event: If the Timers::master_down reached zero
                    Event::MasterDown => {
                        // print information
                        println!(
                            "VR {}.{}.{}.{} for group {} on interface {} - Master VR is declared Down",
                            vr.parameters.vip[0],
                            vr.parameters.vip[1],
                            vr.parameters.vip[2],
                            vr.parameters.vip[3],
                            vr.parameters.vrid,
                            vr.parameters.interface
                        );
                        // get and store vr's interface mac
                        vr.parameters.ifmac = get_mac_addresses(sockfd, &vr, debug);
                        // set VRRP virtual mac address
                        let mut vmac = ETHER_VRRP_V2_SRC_MAC;
                        vmac[5] = vr.parameters.vrid();
                        set_mac_addresses(sockfd, &vr, vmac, debug);
                        // set IP addresses (including VIP) on the vr's interface
                        set_ip_addresses(sockfd, &vr, true, debug);
                        // (re)set IP routes
                        set_ip_routes(sockfd, &vr, true, debug);
                        // send gratuitious ARP requests
                        let arp_sockfd = arp::open_raw_socket_arp().unwrap();
                        arp::broadcast_gratuitious_arp(arp_sockfd, &vr).unwrap();
                        // set advertisement timer
                        vr.timers.advert = vr.parameters.adverint;
                        // send ADVERTISEMENT
                        packets::send_advertisement(sockfd, &vr, debug).unwrap();
                        // print information
                        println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Backup to Master",
                            vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                            vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                        );
                        // transition to Master state
                        States::Master
                    }
                    // event: if Shutdown event is received
                    Event::Shutdown => {
                        // print information
                        println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Backup to Down",
                                vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                                vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                        );
                        // cancel the 'active' master_down timer
                        vr.timers.master_down = std::f32::NAN;
                        // transition to Down state
                        States::Down
                    }
                    _ => {
                        continue;
                    }
                }
            }
            States::Master => {
                match event {
                    // event: Advertisement timer expired in timer thread
                    Event::GenAdvert => {
                        // send ADVERTISEMENT message
                        packets::send_advertisement(sockfd, &vr, debug).unwrap();
                        // reset the advertisement timer to advertisement interval
                        vr.timers.advert = vr.parameters.adverint;
                        continue;
                    }
                    // event: we got an ADVERTISEMENT message
                    Event::Advert(ipsrc, prio) => {
                        // if priority is zero
                        if prio == 0 {
                            // send an ADVERTISEMENT message
                            packets::send_advertisement(sockfd, &vr, debug).unwrap();
                            // reset the advertisement timer to advertisement interval
                            vr.timers.advert = vr.parameters.adverint;
                            // state doesn't change
                            continue;
                        } else {
                            // if ADVERTISEMENT priority is greater than local priority
                            // OR (the priority is equal AND primary address is higher than
                            // local address)
                            if prio > vr.parameters.prio
                                || (prio == vr.parameters.prio
                                    && is_primary_higher(&ipsrc, &vr.parameters.ipaddrs[0]))
                            {
                                // cancel advertisement timer
                                vr.timers.advert = 0;
                                // reset master_down timer to master_down interval
                                vr.timers.master_down = vr.parameters.master_down;
                                // clear down flag (mark master alive)
                                vr.flags.clear_down_flag();
                                // print debugging information
                                print_debug(
                                    debug,
                                    DEBUG_LEVEL_HIGH,
                                    format!("debug(fsm): down flag cleared in Master state"),
                                );
                                // restore interface's MAC address
                                set_mac_addresses(sockfd, &vr, vr.parameters.ifmac, debug);
                                // remove VIP on the vr's interface
                                set_ip_addresses(sockfd, &vr, false, debug);
                                // re-set IP routes
                                set_ip_routes(sockfd, &vr, true, debug);
                                // print information
                                println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Master to Backup",
                                    vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                                    vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                                );
                                // transition to Backup state
                                States::Backup
                            } else {
                                continue;
                            }
                        }
                    }
                    // event: master is declared down by timer thread
                    Event::MasterDown => {
                        // print debugging information
                        print_debug(
                            debug,
                            DEBUG_LEVEL_EXTENSIVE,
                            format!("debug(fsm): received MasterDown event in Master State"),
                        );
                        continue;
                    }
                    // event: if Shutdown event is received
                    Event::Shutdown => {
                        // print information
                        println!("VR {}.{}.{}.{} for group {} on interface {} - Changed from Master to Down",
                                vr.parameters.vip[0], vr.parameters.vip[1], vr.parameters.vip[2],
                                vr.parameters.vip[3], vr.parameters.vrid, vr.parameters.interface
                        );
                        // cancel the 'advert' timer
                        vr.timers.advert = 0;
                        // send ADVERTISEMENT with priority equal 0
                        vr.parameters.prio = 0;
                        packets::send_advertisement(sockfd, &vr, debug).unwrap();
                        // restore interface's MAC address
                        set_mac_addresses(sockfd, &vr, vr.parameters.ifmac, debug);
                        // remove VIP on the vr's interface
                        set_ip_addresses(sockfd, &vr, false, debug);
                        // re-set IP routes
                        set_ip_routes(sockfd, &vr, true, debug);
                        // transition to Down state
                        States::Down
                    }
                    _ => {
                        continue;
                    }
                }
            }
        };
        // print debugging information
        print_debug(
            debug,
            DEBUG_LEVEL_EXTENSIVE,
            format!("debug(worker): worker thread {} released locks", id),
        );
    }
}

// register_tx() function
/// registers the virtual router sending channel
fn register_tx(
    vr: &Arc<RwLock<VirtualRouter>>,
    tx: &Arc<Mutex<mpsc::Sender<Event>>>,
    id: usize,
    debug: &Verbose,
) {
    // print debugging information
    print_debug(
        debug,
        DEBUG_LEVEL_EXTENSIVE,
        format!("debug(worker-reg): acquiring write lock for thread {}", id),
    );
    // acquire write lock on virtual router
    let mut vr = vr.write().unwrap();
    // print debugging information
    print_debug(
        debug,
        DEBUG_LEVEL_EXTENSIVE,
        format!("debug(worker-reg): acquired write lock for thread {}", id),
    );

    // setting up the notification tx channel
    vr.parameters.notification = Option::Some(Arc::clone(tx));
    // print debugging information
    print_debug(
        debug,
        DEBUG_LEVEL_EXTENSIVE,
        format!("debug(worker-reg): registered tx channel for thread {}", id),
    );
}

// is_primary_higher() function
/// return a boolean true if the primary address is higher than local
fn is_primary_higher(primary: &[u8; 4], local: &[u8; 4]) -> bool {
    let mut result = false;
    // do a per byte comparison, and returns true if primary's is higher
    for (i, b) in primary.iter().enumerate() {
        if *b > local[i] {
            result = true;
        }
    }
    result
}

// set_ip_addresses_ioctl() function
/// set or unset IPv4 addresses on a virtual-router interface
fn set_ip_addresses(
    sockfd: i32,
    vr: &std::sync::RwLockWriteGuard<VirtualRouter>,
    vip: bool,
    debug: &Verbose,
) {
    // create addr and netmask vector
    let mut addrs: Vec<[u8; 4]> = Vec::new();
    let mut netmasks: Vec<[u8; 4]> = Vec::new();

    // push vr's address(es) inside addr vector
    for ip in &vr.parameters.ipaddrs {
        addrs.push(*ip);
    }
    // push address' netmasks inside the netmask vector
    for m in &vr.parameters.ipmasks {
        netmasks.push(*m);
    }

    // if true, add vip and netmask to the respective vectors
    // make sure this is done last, so the VIP is on top of the addrs vector
    // otherwise it will never replace the current IP when using ioctls
    if vip == true {
        // add vip to the IP addresses vector
        addrs.push(vr.parameters.vip);
        // add first address' netmask
        netmasks.push(vr.parameters.ipmasks[0]);
    }

    // construct interface name
    let ifname = CString::new(vr.parameters.interface.as_bytes() as &[u8]).unwrap();

    // set the last ip address from vector
    let idx = addrs.len() - 1;

    // print debugging information
    print_debug(
        debug,
        DEBUG_LEVEL_HIGH,
        format!(
            "debug(ip): setting IP address {}.{}.{}.{} netmask {}.{}.{}.{} on {:?}",
            addrs[idx][0],
            addrs[idx][1],
            addrs[idx][2],
            addrs[idx][3],
            netmasks[idx][0],
            netmasks[idx][1],
            netmasks[idx][2],
            netmasks[idx][3],
            ifname
        ),
    );
    if let Err(e) = linux_netdev::set_ip_address(sockfd, &ifname, addrs[idx], netmasks[idx]) {
        eprintln!(
            "error(ip): error while assigning IP address on interface {:?}: {}",
            &ifname, e
        );
    }
}

// get_mac_addresses() function
/// get Ethernet MAC address from vr's interface
fn get_mac_addresses(
    sockfd: i32,
    vr: &std::sync::RwLockWriteGuard<VirtualRouter>,
    debug: &Verbose,
) -> [u8; 6] {
    // construct interface name
    let ifname = CString::new(vr.parameters.interface.as_bytes() as &[u8]).unwrap();

    // get mac address of interface
    match linux_netdev::get_mac_addr(sockfd, &ifname, debug) {
        Ok(mac) => mac,
        Err(e) => {
            eprintln!(
                "error(mac): error while getting MAC address on interface {:?}: {}",
                ifname, e
            );
            [0, 0, 0, 0, 0, 0]
        }
    }
}

// set_mac_addresses() function
/// Set Ethernet MAC address on vr's interface
fn set_mac_addresses(
    sockfd: i32,
    vr: &std::sync::RwLockWriteGuard<VirtualRouter>,
    mac: [u8; 6],
    debug: &Verbose,
) {
    // construct interface name
    let ifname = CString::new(vr.parameters.interface.as_bytes() as &[u8]).unwrap();

    // set mac address
    if let Err(e) = linux_netdev::set_mac_addr(sockfd, &ifname, mac, debug) {
        eprintln!("error(mac): error while setting mac address: {}", e);
    }
}

// set_ip_routes() function
/// set or unset IPv4 routes on virtual-router interfaces
fn set_ip_routes(
    sockfd: i32,
    vr: &std::sync::RwLockWriteGuard<VirtualRouter>,
    set_flag: bool,
    debug: &Verbose,
) {
    // acquire mutex lock on protocols
    let protocols = &vr.parameters.protocols;
    let protocols = protocols.lock().unwrap();

    // for every static routes
    for st in protocols.r#static.as_ref().unwrap() {
        if let Err(e) = set_ip_route(
            sockfd,
            &vr.parameters.interface,
            st.route(),
            st.mask(),
            st.nh(),
            st.metric(),
            st.mtu(),
            set_flag,
            debug,
        ) {
            eprintln!(
                "error(route): cannot set or delete route {:?}: {}",
                st.route(),
                e
            );
        }
    }
}
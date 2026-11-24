//! Servis iskeleti (bölüm 2, 9): her servis kendi iş parçacığında.
//!
//! İş parçacığı COM'u başlatır, durum okuma işlevini (`make` ile iş parçacığı
//! içinde kurulur, COM nesneleri dışarı sızmaz) çağırır, durum değiştiyse
//! önbelleğe yazıp olayı yayınlar. İşlev şu durumlarda yeniden çalışır:
//! - bir komut gelince (`send`),
//! - Windows bir değişiklik bildirince (`Waker::wake`, 7.8: yoklama yerine olay),
//! - yedek yoklama aralığı dolunca (olay kaçarsa diye seyrek).
//!
//! 8.1 tembel servisler: servis kapalı başlar, `set_active(true)` ile `make`
//! çağrılıp işlev kurulur. `set_active(false)` işlevi (ve içindeki Windows
//! aboneliklerini) bırakır, durumu başlangıç değerine döndürür; iş parçacığı
//! yalnızca yeni mesaj bekler, yoklama yapmaz. Yerleşimde olmayan widget'ın
//! servisi böylece hiç işlemci harcamaz (bkz. `services::sync_active`).
//!
//! Okuma hatası `None`/varsayılan durum demektir: servis düşse de dock ayakta
//! kalır, widget "yok" gösterir.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Art arda gelen bildirimleri tek okumada birleştirme süresi.
const COALESCE: Duration = Duration::from_millis(30);

enum Msg<C> {
    Cmd(C),
    Wake,
    Active(bool),
}

/// Olay işleyicilerinden servisi "şimdi oku" diye uyandırır. Herhangi bir iş
/// parçacığından (COM / WinRT geri çağrıları) çağrılabilir; hemen döner.
pub struct Waker<C>(Sender<Msg<C>>);

impl<C> Clone for Waker<C> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C> Waker<C> {
    pub fn wake(&self) {
        let _ = self.0.send(Msg::Wake);
    }
}

pub struct ServiceHandle<S, C> {
    state: Mutex<S>,
    commands: Mutex<Sender<Msg<C>>>,
    active: AtomicBool,
}

impl<S: Clone, C> ServiceHandle<S, C> {
    /// Son bilinen durum (`get_<şey>` komutları için).
    pub fn get(&self) -> S {
        self.state.lock().unwrap().clone()
    }

    /// Kapalı serviste komut yok sayılır.
    pub fn send(&self, cmd: C) {
        let _ = self.commands.lock().unwrap().send(Msg::Cmd(cmd));
    }

    pub fn wake(&self) {
        let _ = self.commands.lock().unwrap().send(Msg::Wake);
    }

    /// Servisi başlat / durdur (8.1). Aynı değer tekrar verilirse bir şey olmaz.
    pub fn set_active(&self, on: bool) {
        if self.active.swap(on, Ordering::SeqCst) != on {
            let _ = self.commands.lock().unwrap().send(Msg::Active(on));
        }
    }
}

/// `make`, servisin iş parçacığında, servis her başlatıldığında çağrılır; aldığı
/// `Waker`'ı Windows olaylarına bağlayabilir. Döndürdüğü işlev bırakılınca
/// (servis durunca) abonelikleri de bırakılmalı. `interval` yedek yoklama aralığıdır.
pub fn spawn<S, C, M, T>(
    app: &AppHandle,
    name: &str,
    event: &'static str,
    interval: Duration,
    initial: S,
    mut make: M,
) where
    S: Clone + PartialEq + Serialize + Send + Sync + 'static,
    C: Send + 'static,
    M: FnMut(Waker<C>) -> T + Send + 'static,
    T: FnMut(Option<C>) -> S,
{
    let (tx, rx) = mpsc::channel::<Msg<C>>();
    let waker = Waker(tx.clone());
    app.manage(ServiceHandle {
        state: Mutex::new(initial.clone()),
        commands: Mutex::new(tx),
        active: AtomicBool::new(false),
    });
    let app = app.clone();
    let label = name.to_string();
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            // WinRT ve Core Audio için.
            crate::win::com_mta();
            let publish = |next: S| {
                let handle = app.state::<ServiceHandle<S, C>>();
                let changed = {
                    let mut state = handle.state.lock().unwrap();
                    if *state != next {
                        *state = next.clone();
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    let _ = app.emit(event, &next);
                }
            };

            let mut tick: Option<T> = None;
            let mut pending: Option<C> = None;
            // Birleştirme sırasında gelen, işlenmeyi bekleyen mesaj.
            let mut carry: Option<Msg<C>> = None;
            // Yeni bir okuma gerekiyor mu (kapalıyken gelen mesajlar okutmaz).
            let mut read = false;
            loop {
                if read && let Some(t) = tick.as_mut() {
                    publish(t(pending.take()));
                }
                read = false;
                let msg = match carry.take() {
                    Some(msg) => msg,
                    // Açıkken yedek yoklama; kapalıyken yalnızca mesaj bekle.
                    None if tick.is_some() => match rx.recv_timeout(interval) {
                        Ok(msg) => msg,
                        Err(RecvTimeoutError::Timeout) => {
                            read = true;
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                    None => match rx.recv() {
                        Ok(msg) => msg,
                        Err(_) => break,
                    },
                };
                match msg {
                    Msg::Cmd(cmd) => {
                        if tick.is_some() {
                            pending = Some(cmd);
                            read = true;
                        }
                    }
                    Msg::Wake => {
                        if tick.is_some() {
                            // Bir değişiklik çoğu zaman birkaç bildirimle gelir: birleştir.
                            std::thread::sleep(COALESCE);
                            while let Ok(msg) = rx.try_recv() {
                                if !matches!(msg, Msg::Wake) {
                                    carry = Some(msg);
                                    break;
                                }
                            }
                            read = true;
                        }
                    }
                    Msg::Active(true) => {
                        if tick.is_none() {
                            log(&label, "başladı");
                            tick = Some(make(waker.clone()));
                            read = true;
                        }
                    }
                    Msg::Active(false) => {
                        if tick.take().is_some() {
                            log(&label, "durdu");
                            pending = None;
                            publish(initial.clone());
                        }
                    }
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("{name} servisi başlatılamadı: {e}");
    }
}

fn log(name: &str, msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("[runner] {name} {msg}");
    }
}

//! NFC Foundry desktop shell. Feature views live in their own crates.

use std::sync::{Arc, Mutex};

use gpui_kit::base::v_flex;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::prelude::*;
use gpui_kit::*;
use nfc_app::Service;
use nfc_app::classic::{BackupStore, KeyStore};
use nfc_classic_gui::ClassicWorkspace;
use nfc_drivers::DriverRegistry;
use nfc_type2_gui::Type2Workspace;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Domain {
    #[default]
    Classic,
    Type2,
}

impl Domain {
    const fn index(self) -> usize {
        match self {
            Self::Classic => 0,
            Self::Type2 => 1,
        }
    }

    const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Type2,
            _ => Self::Classic,
        }
    }
}

struct AppShell {
    domain: Domain,
    classic: Entity<ClassicWorkspace>,
    type2: Entity<Type2Workspace>,
}

impl AppShell {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        service: Arc<Mutex<Service>>,
        keys: KeyStore,
        key_store_path: Option<std::path::PathBuf>,
    ) -> Self {
        let classic = cx.new(|cx| {
            ClassicWorkspace::new(
                window,
                cx,
                service.clone(),
                keys,
                key_store_path,
                BackupStore::default(),
            )
        });
        let type2 =
            cx.new(|cx| Type2Workspace::new(window, cx, service.clone(), BackupStore::default()));
        Self {
            domain: Domain::Classic,
            classic,
            type2,
        }
    }
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        v_flex()
            .size_full()
            .child(
                TabBar::new("domain-tabs")
                    .underline()
                    .selected_index(self.domain.index())
                    .child(Tab::new().label("MIFARE Classic"))
                    .child(Tab::new().label("Type 2 / NDEF"))
                    .on_click(move |index, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.domain = Domain::from_index(*index);
                            cx.notify();
                        });
                    }),
            )
            .child(match self.domain {
                Domain::Classic => self.classic.clone().into_any_element(),
                Domain::Type2 => self.type2.clone().into_any_element(),
            })
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let service = Arc::new(Mutex::new(Service::with_transport(Arc::new(
                DriverRegistry,
            ))));
            let key_store_path = KeyStore::default_path();
            let keys = key_store_path
                .as_ref()
                .and_then(|path| KeyStore::load_or_default(path).ok())
                .unwrap_or_default();
            gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                let service = service.clone();
                let keys = keys.clone();
                let key_store_path = key_store_path.clone();
                cx.new(|cx| AppShell::new(window, cx, service, keys, key_store_path))
            })
            .expect("failed to open the NFC Foundry window");
        });
}

// nativis-gui/src/main.rs
//
// Sketsa Settings Panel Nativis menggunakan Iced.
// Fokus: manajemen Media Plugin (video/image/html backend)
// dan Platform Layer (kde-x11, windows, macos) — install & uninstall.
//
// Catatan integrasi:
// - fetch_plugin_registry / install_plugin / uninstall_plugin adalah stub.
//   Sambungkan ke nativis-plugin::resolver untuk enumerasi & registrasi nyata.
// - Untuk Platform kind (mis. kde-x11), install/uninstall adalah tempat yang
//   tepat untuk menjalankan langkah KPackage install/remove + setup
//   QML2_IMPORT_PATH, supaya user tidak perlu sudo manual.
// - Saat uninstall Platform plugin yang sedang aktif, pastikan juga
//   membersihkan per-monitor SHM instance guard terkait sebelum unregister.

use iced::widget::{button, column, container, row, scrollable, text, Column};
use iced::{Alignment, Element, Length, Task};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginKind {
    Media,    // video/image/html decoder backend plugin
    Platform, // kde-x11, (future) windows, macos
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    pub installed: bool,
    pub busy: bool, // sedang install/uninstall, dipakai untuk disable tombol
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    MediaPlugins,
    PlatformLayers,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    RefreshList,
    ListRefreshed(Vec<PluginEntry>),
    Install(String),
    Uninstall(String),
    InstallFinished(String, Result<(), String>),
    UninstallFinished(String, Result<(), String>),
}

pub struct SettingsPanel {
    active_tab: Tab,
    plugins: Vec<PluginEntry>,
    status_message: Option<String>,
}

impl SettingsPanel {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                active_tab: Tab::MediaPlugins,
                plugins: Vec::new(),
                status_message: None,
            },
            Task::perform(fetch_plugin_registry(), Message::ListRefreshed),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TabSelected(tab) => {
                self.active_tab = tab;
                Task::none()
            }

            Message::RefreshList => {
                Task::perform(fetch_plugin_registry(), Message::ListRefreshed)
            }

            Message::ListRefreshed(entries) => {
                self.plugins = entries;
                Task::none()
            }

            Message::Install(id) => {
                if let Some(p) = self.plugins.iter_mut().find(|p| p.id == id) {
                    p.busy = true;
                }
                Task::perform(install_plugin(id.clone()), move |res| {
                    Message::InstallFinished(id.clone(), res)
                })
            }

            Message::Uninstall(id) => {
                if let Some(p) = self.plugins.iter_mut().find(|p| p.id == id) {
                    p.busy = true;
                }
                Task::perform(uninstall_plugin(id.clone()), move |res| {
                    Message::UninstallFinished(id.clone(), res)
                })
            }

            Message::InstallFinished(id, res) => {
                if let Some(p) = self.plugins.iter_mut().find(|p| p.id == id) {
                    p.busy = false;
                    match res {
                        Ok(()) => {
                            p.installed = true;
                            self.status_message = Some(format!("{} installed", p.name));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Install gagal: {e}"));
                        }
                    }
                }
                Task::none()
            }

            Message::UninstallFinished(id, res) => {
                if let Some(p) = self.plugins.iter_mut().find(|p| p.id == id) {
                    p.busy = false;
                    match res {
                        Ok(()) => {
                            p.installed = false;
                            self.status_message = Some(format!("{} uninstalled", p.name));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("Uninstall gagal: {e}"));
                        }
                    }
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let tabs = row![
            tab_button("Media Plugins", Tab::MediaPlugins, self.active_tab),
            tab_button("Platform Layers", Tab::PlatformLayers, self.active_tab),
        ]
        .spacing(8);

        let kind_filter = match self.active_tab {
            Tab::MediaPlugins => PluginKind::Media,
            Tab::PlatformLayers => PluginKind::Platform,
        };

        let list: Column<Message> = self
            .plugins
            .iter()
            .filter(|p| p.kind == kind_filter)
            .fold(Column::new().spacing(10), |col, plugin| {
                col.push(plugin_row(plugin))
            });

        let status: Element<Message> = match &self.status_message {
            Some(msg) => text(msg).size(13).into(),
            None => text("").into(),
        };

        container(
            column![tabs, scrollable(list).height(Length::Fill), status,]
                .spacing(16)
                .padding(20),
        )
        .into()
    }
}

fn tab_button(label: &str, tab: Tab, active: Tab) -> Element<'static, Message> {
    let btn = button(text(label.to_string()));
    if tab == active {
        btn.into() // TODO: styling aktif berbeda (mis. warna aksen)
    } else {
        btn.on_press(Message::TabSelected(tab)).into()
    }
}

fn plugin_row(plugin: &PluginEntry) -> Element<'static, Message> {
    let action_button: Element<'static, Message> = if plugin.busy {
        button(text("...")).into()
    } else if plugin.installed {
        button(text("Uninstall"))
            .on_press(Message::Uninstall(plugin.id.clone()))
            .into()
    } else {
        button(text("Install"))
            .on_press(Message::Install(plugin.id.clone()))
            .into()
    };

    row![
        column![
            text(plugin.name.clone()).size(16),
            text(format!("v{}", plugin.version)).size(12),
        ]
        .width(Length::Fill),
        action_button,
    ]
    .align_y(Alignment::Center)
    .into()
}

// --- Backend hooks: sambungkan ke nativis-plugin yang sesungguhnya ---

async fn fetch_plugin_registry() -> Vec<PluginEntry> {
    // TODO: panggil nativis-plugin::registry untuk enumerasi plugin
    // yang tersedia + terinstall (media backend maupun platform layer).
    // Hormati urutan resolver (ingat kasus stub attach() priority 90
    // yang pernah diam-diam memblokir handler asli — audit ulang saat
    // menambah entry point registry baru).
    vec![]
}

async fn install_plugin(id: String) -> Result<(), String> {
    // TODO:
    // - Media plugin: download package, verifikasi checksum/signature,
    //   lalu daftarkan lewat nativis-plugin::resolver.
    // - Platform plugin (mis. kde-x11): jalankan langkah KPackage install,
    //   set QML2_IMPORT_PATH ke ~/.local/lib/qt5/qml, lalu restart
    //   plasmashell secara terprogram dari sisi Rust (hindari sudo manual).
    let _ = id;
    Ok(())
}

async fn uninstall_plugin(id: String) -> Result<(), String> {
    // TODO:
    // - Unregister dari resolver.
    // - Jika Platform plugin: hapus KPackage, dan pastikan bersihkan
    //   per-monitor SHM instance guard yang masih terkait sebelum
    //   benar-benar melepas binding-nya.
    let _ = id;
    Ok(())
}

pub fn main() -> iced::Result {
    iced::application("Nativis Settings", SettingsPanel::update, SettingsPanel::view)
        .run_with(SettingsPanel::new)
}

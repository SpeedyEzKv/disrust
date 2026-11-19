#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod avatar;
mod discord;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

use eframe::egui::{
    self, Color32, Context, Frame, Margin, RichText, ScrollArea, Stroke, TextEdit, TextureHandle,
};

use crate::discord::Account;

const INK: Color32 = Color32::from_rgb(33, 46, 43);
const MUTED: Color32 = Color32::from_rgb(89, 105, 98);
const ACCENT: Color32 = Color32::from_rgb(35, 110, 82);
const PAGE: Color32 = Color32::from_rgb(247, 249, 246);
const LINE: Color32 = Color32::from_rgb(220, 230, 222);
const SIDEBAR: Color32 = Color32::from_rgb(234, 241, 235);

#[derive(Default, PartialEq)]
enum View {
    #[default]
    Account,
    Demo,
    Server(usize),
}

#[derive(Default)]
struct DisRust {
    view: View,
    account: Option<Account>,
    avatar: Option<TextureHandle>,
    pending: Option<PendingLogin>,
    error: Option<String>,
    filter: String,
    draft: String,
    demo_messages: Vec<String>,
}

struct PendingLogin {
    result: Receiver<Result<Account, String>>,
    cancelled: Arc<AtomicBool>,
}

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([970.0, 660.0])
        .with_min_inner_size([660.0, 440.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/avatar.png")) {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "DisRust",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|creation| {
            let mut style = (*creation.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::light();
            style.visuals.override_text_color = Some(INK);
            style.visuals.selection.bg_fill = Color32::from_rgb(186, 224, 204);
            style.visuals.widgets.active.bg_fill = Color32::from_rgb(204, 232, 215);
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(215, 237, 223);
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            creation.egui_ctx.set_style(style);
            Ok(Box::new(DisRust::default()))
        }),
    )
}

impl DisRust {
    fn start_login(&mut self, ctx: &Context) {
        let (sender, result) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = Arc::clone(&cancelled);
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = discord::connect(&cancellation);
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.pending = Some(PendingLogin { result, cancelled });
        self.error = None;
    }

    fn poll_login(&mut self, ctx: &Context) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Подключение неожиданно завершилось.".into())
            }
        };
        let cancelled = pending.cancelled.load(Ordering::Relaxed);
        self.pending = None;
        if cancelled {
            self.error = Some("Вход отменён.".into());
            return;
        }

        match result {
            Ok(mut account) => {
                self.avatar = account.avatar.take().map(|image| {
                    let size = [image.width() as usize, image.height() as usize];
                    ctx.load_texture(
                        "discord-avatar",
                        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                        egui::TextureOptions::LINEAR,
                    )
                });
                self.account = Some(account);
                self.view = View::Account;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn sidebar(&mut self, ctx: &Context) {
        egui::SidePanel::left("navigation")
            .exact_width(226.0)
            .resizable(false)
            .frame(Frame::new().fill(SIDEBAR).inner_margin(Margin::same(18)))
            .show(ctx, |ui| {
                ui.label(RichText::new("DisRust").size(29.0).strong().color(INK));
                ui.add_space(17.0);
                if nav_button(ui, self.view == View::Account, "Аккаунт") {
                    self.view = View::Account;
                }
                if nav_button(ui, self.view == View::Demo, "Демо чата") {
                    self.view = View::Demo;
                }
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);
                ui.label(RichText::new("Серверы").size(14.0).strong().color(MUTED));
                let Some(account) = &self.account else {
                    ui.label(RichText::new("Появятся после входа").color(MUTED));
                    return;
                };
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text("Поиск сервера")
                        .desired_width(f32::INFINITY),
                );
                let search = self.filter.trim().to_lowercase();
                ScrollArea::vertical().id_salt("guilds").show(ui, |ui| {
                    let mut shown = 0;
                    for (index, guild) in account.guilds.iter().enumerate() {
                        if !guild.name.to_lowercase().contains(&search) {
                            continue;
                        }
                        shown += 1;
                        if nav_button(ui, self.view == View::Server(index), &guild.name) {
                            self.view = View::Server(index);
                        }
                    }
                    if shown == 0 {
                        ui.label(if search.is_empty() {
                            "Серверов пока нет"
                        } else {
                            "Ничего не найдено"
                        });
                    }
                });
            });
    }

    fn account_view(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        heading(ui, "Аккаунт Discord");
        ui.add_space(15.0);
        card().show(ui, |ui| {
            if let Some(account) = &self.account {
                ui.horizontal(|ui| {
                    if let Some(avatar) = &self.avatar {
                        ui.add(
                            egui::Image::new((avatar.id(), egui::vec2(52.0, 52.0)))
                                .corner_radius(12),
                        );
                    }
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(
                                account
                                    .user
                                    .global_name
                                    .as_deref()
                                    .unwrap_or(&account.user.username),
                            )
                            .size(21.0)
                            .strong(),
                        );
                        ui.label(RichText::new(format!("@{}", account.user.username)).color(MUTED));
                    });
                });
                ui.add_space(4.0);
                ui.label(format!("Серверов в списке: {}", account.guilds.len()));
                if ui.button("Выйти из DisRust").clicked() {
                    self.account = None;
                    self.avatar = None;
                    self.filter.clear();
                }
            } else if let Some(pending) = &self.pending {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Ожидаем подтверждения входа в браузере…");
                });
                let active = !pending.cancelled.load(Ordering::Relaxed);
                if ui
                    .add_enabled(active, egui::Button::new("Отменить вход"))
                    .clicked()
                {
                    pending.cancelled.store(true, Ordering::Relaxed);
                }
            } else {
                ui.label("Подключи Discord, чтобы увидеть свой профиль и серверы.");
                let button = egui::Button::new(
                    RichText::new("Войти через Discord")
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(ACCENT)
                .min_size(egui::vec2(156.0, 34.0));
                if ui.add(button).clicked() {
                    self.start_login(ctx);
                }
            }
        });
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(Color32::from_rgb(157, 53, 45)));
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Подключение даёт доступ к профилю и списку серверов. Реальные переписки и звонки здесь недоступны.").color(MUTED));
    }

    fn demo_view(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Демо чата");
        ui.label(
            RichText::new(
                "Локальный просмотр. Сообщения остаются в этом окне и исчезают при выходе.",
            )
            .color(MUTED),
        );
        ui.add_space(10.0);
        card().show(ui, |ui| {
            ui.label(RichText::new("Тестовый канал").strong().size(17.0));
            ui.separator();
            let history_height = (ui.available_height() - 87.0).max(58.0);
            ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(history_height)
                .min_scrolled_height(history_height)
                .show(ui, |ui| {
                    ui.label(RichText::new("DisRust").strong().color(ACCENT));
                    ui.label("Попробуй написать сообщение, чтобы проверить редактор.");
                    for message in &self.demo_messages {
                        ui.add_space(10.0);
                        ui.label(RichText::new("Ты").strong());
                        ui.label(message);
                    }
                });
            ui.separator();
            let mut send = false;
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 110.0).max(100.0);
                let input = ui.add(
                    TextEdit::singleline(&mut self.draft)
                        .hint_text("Сообщение для демо")
                        .desired_width(width)
                        .char_limit(2000),
                );
                let enter =
                    input.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let clicked = ui
                    .add_enabled(!self.draft.trim().is_empty(), egui::Button::new("Добавить"))
                    .clicked();
                send = enter || clicked;
                if enter {
                    input.request_focus();
                }
            });
            if send && !self.draft.trim().is_empty() {
                self.demo_messages.push(self.draft.trim().to_owned());
                self.draft.clear();
            }
        });
    }
}

impl eframe::App for DisRust {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        self.poll_login(ctx);
        self.sidebar(ctx);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(PAGE).inner_margin(Margin::same(28)))
            .show(ctx, |ui| match self.view {
                View::Account => self.account_view(ui, ctx),
                View::Demo => self.demo_view(ui),
                View::Server(index) => {
                    if let Some(guild) = self.account.as_ref().and_then(|account| account.guilds.get(index)) {
                        heading(ui, &guild.name);
                        ui.add_space(15.0);
                        card().show(ui, |ui| {
                            ui.label("Discord OAuth не предоставляет каналы и переписки при этом типе входа.");
                        });
                    }
                }
            });
    }
}

fn heading(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(29.0).strong());
}

fn card() -> Frame {
    Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(10)
        .inner_margin(Margin::same(18))
}

fn nav_button(ui: &mut egui::Ui, selected: bool, label: &str) -> bool {
    ui.add_sized(
        [ui.available_width(), 32.0],
        egui::Button::selectable(selected, label).truncate(),
    )
    .clicked()
}

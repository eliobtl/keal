use std::{os::unix::process::CommandExt, sync::mpsc::{channel, Receiver, Sender, TryRecvError}};

use fork::{fork, Fork};
// use iced::{Application, executor, Command, widget::{row as irow, text_input, column as icolumn, container, text, Space, scrollable, button, image, svg}, font, Element, Length, subscription, Event, keyboard::{self, KeyCode, Modifiers}, futures::channel::mpsc};
use macroquad::{miniquad::{window::set_mouse_cursor, CursorIcon}, prelude::*};
use nucleo_matcher::Matcher;
use smallvec::SmallVec;

use crate::{icon::{IconCache, Icon}, config::config, plugin::{Action, entry::{Label, OwnedEntry}}, log_time};

pub use styled::Theme;
// use styled::{ButtonStyle, TextStyle};

use self::{match_span::MatchSpan, async_manager::AsyncManager};

mod styled;
mod match_span;
mod async_manager;

/// Returns a vector of indices (byte offsets) at which the text should wrap, as well as the total height of the text
fn measure_text_wrap(text: &str, max_width: f32, font: Option<&Font>, font_size: f32, line_height: f32) -> WrapInfo {
    let max_width = max_width.max(font_size*2.0);

    let mut splits = SmallVec::new();
    let mut height = font_size;

    let mut running_width = 0.0;

    let mut line_start = 0;
    let mut last = 0;
    let mut iter = text.char_indices();
    iter.next();
    for (index, c) in iter {
        let dims = measure_text(&text[last..index], font, font_size as u16, 1.0);

        if c == '\n' || running_width + dims.width >= max_width {
            line_start = index;
            running_width = 0.0;

            height += font_size + line_height;
            splits.push(last);
        } 

        running_width += dims.width;
        last = index;
    }

    if line_start < text.len() {
        let dims = measure_text(&text[last..], font, font_size as u16, 1.0);
        running_width += dims.width;

        splits.push(text.len());
    }

    let width = if line_start == 0 { running_width } else { max_width };

    WrapInfo { splits, width, height }
}

struct WrapInfo {
    splits: SmallVec<[usize; 8]>,
    width: f32,
    height: f32
}

#[derive(Default)]
struct Entries {
    list: Vec<OwnedEntry>,
    /// info for entry.name and entry.comment (optional)
    wrap_info: Vec<(WrapInfo, Option<WrapInfo>)>,
    total_height: f32
}

impl Entries {
    fn new(list: Vec<OwnedEntry>, font: Option<&Font>) -> Self {
        let mut this = Self {
            list,
            wrap_info: Vec::new(),
            total_height: 0.0
        };

        this.recalculate(font);
        this
    }

    /// call this when the screen width changes
    fn recalculate(&mut self, font: Option<&Font>) {
        let config = config();

        self.total_height = 0.0;
        self.wrap_info.clear();
        self.wrap_info.extend(self.list.iter().map(|entry| {
            let name = measure_text_wrap(&entry.name, screen_width()/2.0, font, config.font_size, 5.0);
            let mut max_height = name.height;

            let comment_width = screen_width() - name.width - 10.0 - 20.0 - 10.0; // this removes: name left padding, name-comment inner padding, comment right padding
            let comment = entry.comment.as_ref()
                .map(|comment| measure_text_wrap(comment, comment_width, font, config.font_size, 5.0))
                .inspect(|comment| max_height = max_height.max(comment.height));

            self.total_height += max_height + 20.0;

            (name, comment)
        }));
    }
}

pub struct Keal {
    // UI state
    input: String,
    selected: usize,
    scroll: f32,

    old_screen_width: f32,

    // data state
    icons: IconCache,
    font: Option<Font>,

    entries: Entries,
    manager: AsyncManager,

    message_sender: Sender<Message>,
    message_rec: Receiver<Message>
}

#[derive(Debug, Clone)]
pub enum Message {
    // UI events
    TextInput(String),
    Launch(Option<Label>),

    // Worker events
    IconCacheLoaded(IconCache),
    Entries(Vec<OwnedEntry>),
    Action(Action)
}

impl Keal {
    pub fn new() -> Self {
        log_time("initializing app");

        let config = config();

        let (message_sender, message_rec) = channel();

        let iosevka = include_bytes!("../../public/iosevka-regular.ttf");
        let iosevka = load_ttf_font_from_bytes(iosevka).expect("failed to load font");
        log_time("finished loading font");

        {
            let message_sender = message_sender.clone();
            std::thread::spawn(move || {
                let icon_cache = IconCache::new(&config.icon_theme);
                let _ = message_sender.send(Message::IconCacheLoaded(icon_cache));
            });
        }

        let manager = AsyncManager::new(Matcher::default(), 50, true, message_sender.clone());

        log_time("finished initializing");

        Keal {
            input: String::new(),
            selected: 0,
            scroll: 0.0,
            old_screen_width: 0.0,
            icons: Default::default(),
            font: Some(iosevka),
            entries: Default::default(),
            manager,
            message_sender,
            message_rec
        }
    }

    pub fn render(&mut self) {
        let entries = &self.entries;
        let config = config();

        let font = self.font.as_ref();
        let font_size = config.font_size as u16;
        // let font_size_ratio = config.font_size / config.font_size.floor();

        let data = &mut *self.manager.get_data();
        let mut buf = vec![];

        // TODO: scrollbar

        let search_bar_height = (config.font_size*3.25).ceil();

        self.scroll += mouse_wheel().1*20.0;
        self.scroll = clamp(self.scroll, screen_height()-self.entries.total_height - search_bar_height, 0.0);

        let mut offset_y = search_bar_height + self.scroll;

        set_mouse_cursor(CursorIcon::Default);
        for (index, (entry, wrap_info)) in entries.list.iter().zip(entries.wrap_info.iter()).enumerate() {
            let max_height = wrap_info.0.height.max(wrap_info.1.as_ref().map(|x| x.height).unwrap_or(0.0));
            let next_offset_y = offset_y + max_height + 20.0;
            if next_offset_y < 0.0 { 
                offset_y = next_offset_y;
                continue
            }
            if offset_y > screen_height() { break }

            let selected = self.selected == index;

            let (_, mouse_y) = mouse_position();
            if mouse_y >= offset_y && mouse_y < next_offset_y {
                set_mouse_cursor(CursorIcon::Pointer);
                if !selected {
                    draw_rectangle(0.0, offset_y, screen_width(), next_offset_y-offset_y, config.theme.hovered_choice_background);
                }
            }
            if selected {
                draw_rectangle(0.0, offset_y, screen_width(), next_offset_y-offset_y, config.theme.selected_choice_background);
            } 

            // if let Some(icon) = &entry.icon {
            //     if let Some(icon) = self.icons.get(icon) {
            //         let element: Element<_, _> = match icon {
            //             Icon::Svg(path) => svg(svg::Handle::from_path(path)).width(config.font_size).height(config.font_size).into(),
            //             Icon::Other(path) => image(path).width(config.font_size).height(config.font_size).into()
            //         };
            //         item = item.push(container(element).padding(4));
            //     }
            // }

            let mut line_start = 0;
            let mut name_offset_y = offset_y + 10.0;

            for &line_end in &wrap_info.0.splits {
                let text = &entry.name[line_start..line_end];

                let mut offset = 10.0;
                for (span, highlighted) in MatchSpan::new(text, &mut data.matcher, &data.pattern, &mut buf) {
                    let dims = measure_text(span, None, config.font_size as u16, 1.0);

                    let color = match highlighted {
                        false => config.theme.text,
                        true => match selected {
                            false => config.theme.matched_text,
                            true => config.theme.selected_matched_text
                        }
                    };

                    draw_text_ex(span, offset, (name_offset_y + config.font_size).ceil(), TextParams { font, font_size, color, ..Default::default() });
                    offset += dims.width;
                }

                name_offset_y += config.font_size + 5.0;
                line_start = line_end;
            }


            let mut comment_offset_y = offset_y + 10.0;
            // fill the whole line up
            if let Some(comment) = &entry.comment {
                let wrap_info = wrap_info.1.as_ref().unwrap();

                let mut line_start = 0;
                for &line_end in &wrap_info.splits {
                    let text = &comment[line_start..line_end];

                    draw_text_ex(text, screen_width() - wrap_info.width - 10.0, comment_offset_y + config.font_size, TextParams { font, font_size, color: config.theme.comment, ..Default::default() });
                    comment_offset_y += config.font_size + 5.0;
                    line_start = line_end;
                }
            }

            offset_y = next_offset_y;

            // .on_press(Message::Launch(Some(entry.label)))
        }

        let height = (config.font_size * 3.25).ceil();
        let text = if self.input.is_empty() { &config.placeholder_text } else { &self.input };

        let size = (config.font_size*1.25) as u16;
        let dims = measure_text(text, None, size, 1.0);

        draw_rectangle(0.0, 0.0, screen_width(), height, config.theme.input_background);
        draw_text_ex(&text, config.font_size, height/2.0 - dims.offset_y + dims.height, TextParams { font, font_size: size, color: config.theme.text, ..Default::default() });
    }

    pub fn update(&mut self) {
        if self.old_screen_width != screen_width() {
            self.entries.recalculate(self.font.as_ref());
            self.old_screen_width = screen_width();
        }

        // KeyPressed { key_code: KeyCode::Escape, .. } => return iced::window::close(),
        let ctrl = is_key_down(KeyCode::LeftControl);
        if is_key_pressed(KeyCode::Down) || (ctrl && is_key_pressed(KeyCode::J)) || (ctrl && is_key_pressed(KeyCode::N)) {
            // TODO: gently scroll window to selected choice
            self.selected += 1;
            self.selected = self.selected.min(self.entries.list.len().saturating_sub(1));
        }
        if is_key_pressed(KeyCode::Up) || (ctrl && is_key_pressed(KeyCode::K)) || (ctrl && is_key_pressed(KeyCode::P)) {
            self.selected = self.selected.saturating_sub(1);
        }

        let message = match self.message_rec.try_recv() {
            Ok(message) => message,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => panic!("manager channel disconnected")
        };

        match message {
            Message::TextInput(input) => self.update_input(input, true),
            Message::Launch(selected) => {
                self.manager.send(async_manager::Event::Launch(selected));
            }
            Message::IconCacheLoaded(icon_cache) => self.icons = icon_cache,
            Message::Entries(entries) => self.entries = Entries::new(entries, self.font.as_ref()),
            Message::Action(action) => return self.handle_action(action),
        };
    }
}

impl Keal {
    pub fn update_input(&mut self, input: String, from_user: bool) {
        self.input = input.clone();
        self.manager.send(async_manager::Event::UpdateInput(input, from_user));
    }

    fn handle_action(&mut self, action: Action) /* -> Command<Message> */ {
        match action {
            Action::None => (),
            Action::ChangeInput(new) => {
                self.manager.with_manager(|m| m.kill());
                self.update_input(new, false);
                // return text_input::move_cursor_to_end(text_input::Id::new("query_input"));
            }
            Action::ChangeQuery(new) => {
                let new = self.manager.use_manager(|m| m.current().map(
                    |plugin| format!("{} {}", plugin.prefix, new) 
                )).unwrap_or(new);
                self.update_input(new, false);

                // return text_input::move_cursor_to_end(text_input::Id::new("query_input"));
            }
            Action::Exec(mut command) => {
                let _ = command.0.exec();
                // return iced::window::close();
            }
            Action::PrintAndClose(message) => {
                println!("{message}");
                // return iced::window::close();
            }
            Action::Fork => match fork().expect("failed to fork") {
                Fork::Parent(_) => (),//return iced::window::close(),
                Fork::Child => ()
            }
            Action::WaitAndClose => {
                self.manager.with_manager(|m| m.wait());
                // return iced::window::close();
            }
        }
    }
}

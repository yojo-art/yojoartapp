
use std::{io::Read, sync::Arc};

use eframe::{egui, NativeOptions};

use egui::{Color32, FontData, FontFamily, ScrollArea, Widget};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;

use crate::{data_model::{self, Visibility}, delay_assets, load_misskey, ConfigFile, LocaleFile, StateFile};

use super::utils::ZoomMediaView;

pub(crate) fn open<F>(options:NativeOptions,ime_show:F)where F:FnMut(&mut bool)+'static{
	/*
	let emoji_src=include_str!("unicodeemoji.txt").split("\n");
	let mut f=std::fs::File::create("unicodeemoji.utf32").unwrap();
	f.write_all(&[00,00,0xFE,0xFF]).unwrap();
	for emoji in emoji_src{
		let c=u32::from_str_radix(&emoji[2..],16).unwrap();
		f.write_all(&c.to_be_bytes()).unwrap();
		//let c=char::from_u32(c).unwrap();
	}
	drop(f);
	*/
	let config=crate::load_config();
	let locale=crate::load_locale();
	let (assets,assets_recv)=tokio::sync::mpsc::channel(10);
	let (note_ui,recv)=tokio::sync::mpsc::channel(4);
	let (reload,reload_recv)=tokio::sync::mpsc::channel(1);
	let (emojis_send,emojis_recv)=tokio::sync::mpsc::channel(1);
	let config0=config.1.clone();
	let client=Client::new();
	let client0=client.clone();
	let assets0=assets.clone();
	std::thread::spawn(move||{
		let rt=tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
		rt.block_on(load_misskey::load_misskey(config0,note_ui,assets0,client0,reload_recv,emojis_send))
	});
	let dummy=data_model::UrlImage::dummy();
	eframe::run_native(
		"YojoArtApp",
		options,
		Box::new(move|cc|{
			// This gives us image support:
			let mut fonts = egui::FontDefinitions::default();
			{
				let mut gz=flate2::read::GzDecoder::new(std::io::Cursor::new(include_bytes!("../include/NotoSansJP-Medium.ttf.gz")));
				let mut buf=vec![];
				gz.read_to_end(&mut buf).unwrap();
				fonts.font_data.insert(
					"notosansjp".to_owned(),
					FontData::from_owned(buf),
				);
				fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "notosansjp".to_owned());
				fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "notosansjp".to_owned());
			}
			let themify={
				let mut gz=flate2::read::GzDecoder::new(std::io::Cursor::new(include_bytes!("../include/themify.ttf.gz")));
				let mut buf=vec![];
				gz.read_to_end(&mut buf).unwrap();
				fonts.font_data.insert(
					"themify".to_owned(),
					FontData::from_owned(buf),
				);
				let themify=egui::FontFamily::Name("themify".into());
				fonts.families.insert(themify.clone(),vec!["themify".to_owned(),"notosansjp".to_owned()]);
				themify
			};
			cc.egui_ctx.set_fonts(fonts);
			let config0=config.1.clone();
			tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(dummy.load_gpu(&cc.egui_ctx,&config0));
			let ctx=cc.egui_ctx.clone();
			let config0=config.1.clone();
			let client0=client.clone();
			std::thread::spawn(||{
				let rt=tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
				rt.block_on(delay_assets::delay_assets(assets_recv,ctx,client0,config0));
			});
			let state=StateFile::load().unwrap_or_default();
			let open_timeline=std::sync::Mutex::new(Some((Some(state.timeline.clone()),state.until_id.clone())));
			Box::new(MainUI{
				config,
				locale,
				input_text:String::new(),
				emojis:None,
				reaction_table:vec![],
				emojis_recv,
				reaction_picker:std::sync::Mutex::new(None),
				show_ime:false,
				button_handle:Box::new(ime_show),
				notes:vec![],
				rcv:recv,
				dummy,
				animate_frame:0u64,
				delay_assets:assets,
				show_cw:std::sync::Mutex::new(None),
				reload,
				client,
				themify,
				auto_update:false,
				view_media:std::sync::Mutex::new(None),
				view_license:false,
				view_config:false,
				view_old_timeline:0f32,
				open_timeline,
				state,
				rn_dialog:std::sync::Mutex::new(None),
			})
		}),
	).unwrap();
}
pub(super) struct MainUI<F>{
	pub(super) config:(String, Arc<ConfigFile>),
	pub(super) locale:Arc<LocaleFile>,
	pub(super) emojis:Option<data_model::EmojiCache>,
	pub(super) reaction_table:Vec<data_model::LocalEmojis>,
	pub(super) emojis_recv:Receiver<data_model::EmojiCache>,
	pub(super) reaction_picker:std::sync::Mutex<Option<String>>,
	pub(super) input_text:String,
	pub(super) show_ime:bool,
	pub(super) button_handle: Box<F>,
	pub(super) notes:Vec<Arc<data_model::Note>>,
	pub(super) rcv:Receiver<Arc<data_model::Note>>,
	pub(super) dummy: data_model::UrlImage,
	pub(super) animate_frame:u64,
	pub(super) delay_assets:tokio::sync::mpsc::Sender<data_model::DelayAssets>,
	pub(super) show_cw:std::sync::Mutex<Option<String>>,
	pub(super) reload: tokio::sync::mpsc::Sender<load_misskey::LoadSrc>,
	pub(super) client: Client,
	pub(super) themify:egui::FontFamily,
	pub(super) auto_update:bool,
	pub(super) view_media:std::sync::Mutex<Option<ZoomMediaView>>,
	pub(super) view_license:bool,
	pub(super) view_config:bool,
	pub(super) view_old_timeline:f32,
	pub(super) open_timeline:std::sync::Mutex<Option<(Option<load_misskey::TimeLine>,Option<String>)>>,
	pub(super) state:StateFile,
	pub(super) rn_dialog:std::sync::Mutex<Option<(String,Visibility)>>,
}
impl <F> eframe::App for MainUI<F> where F:FnMut(&mut bool)+'static{
	fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
		if self.config.1.is_animation.unwrap_or(data_model::DEFAULT_ANIMATION){
			ctx.request_repaint();
			self.animate_frame=chrono::Utc::now().timestamp_millis() as u64;
		}
		if let Ok(emoji)=self.emojis_recv.try_recv(){
			self.reaction_table.clear();
			let mut local_emojis=vec![];
			for id in emoji.local_emojis.iter(){
				local_emojis.push(id);
			}
			local_emojis.sort();
			for (id,url) in local_emojis{
				self.reaction_table.push(data_model::LocalEmojis::InstanceLocal(id.clone(),url.clone()));
			}
			let unicode_emojis=data_model::UnicodeEmoji::load_all();
			for c in unicode_emojis{
				self.reaction_table.push(data_model::LocalEmojis::Unicode(c));
			}
			self.emojis=Some(emoji);
		}
		egui::CentralPanel::default().show(ctx, |ui| {
			if let Ok(mut lock)=self.view_media.lock(){
				if lock.is_some(){
					self.media(ui,&mut lock);
					return;
				}
			}
			ui.add_space(self.config.1.top.unwrap_or(0) as f32);
			if self.view_license{
				self.license(ui,ctx);
				return;
			}
			if self.view_config{
				self.config(ui,ctx);
				return;
			}
			self.timeline(ui,ctx);
		});
	}
}
impl <F> MainUI<F>{
	fn config(&mut self,ui:&mut egui::Ui,ctx:&egui::Context){
		if ui.button(&self.locale.close_settings).clicked(){
			self.view_config=false;
			ctx.request_repaint();
			return;
		}
		if ui.checkbox(&mut self.state.nsfw_always_show,&self.locale.nsfw_always_show).changed(){
			self.state.write(&self.delay_assets);
		}
		if ui.button(&self.locale.show_license).clicked(){
			self.view_license=true;
			ctx.request_repaint();
			return;
		}
		if ui.checkbox(&mut self.state.auto_old_timeline,&self.locale.auto_old_timeline).changed(){
			self.state.write(&self.delay_assets);
		}
		ui.vertical(|ui|{
			ui.heading(&self.locale.thumbnail_mode);
			let old=self.state.file_thumbnail_mode.clone();
			ui.radio_value(&mut self.state.file_thumbnail_mode,crate::FileThumbnailMode::None,&self.locale.no_thumbnail_img);
			ui.radio_value(&mut self.state.file_thumbnail_mode,crate::FileThumbnailMode::Original,&self.locale.always_original_img);
			ui.radio_value(&mut self.state.file_thumbnail_mode,crate::FileThumbnailMode::Thumbnail,&self.locale.default_thumbnail_img);
			if old!=self.state.file_thumbnail_mode{
				self.state.write(&self.delay_assets);
				fn load_img<F>(s:&MainUI<F>,n:&data_model::Note){
					if let Some(q)=n.quote.as_ref(){
						load_img(s,q);
					}
					for f in &n.files{
						match s.state.file_thumbnail_mode{
							crate::FileThumbnailMode::Thumbnail => {
								if let Some(img)=f.img.as_ref(){
									if !img.loaded(){
										let _=s.delay_assets.blocking_send(data_model::DelayAssets::Image(img.clone()));
									}
								}
							},
							crate::FileThumbnailMode::Original => {
								if let Some(img)=f.original_img.as_ref(){
									if !img.loaded(){
										let _=s.delay_assets.blocking_send(data_model::DelayAssets::Image(img.clone()));
									}
								}
							},
							crate::FileThumbnailMode::None => {},
						}
					}
				}
				for n in &self.notes{
					load_img(&self,n);
					//let _=self.delay_assets.blocking_send(data_model::DelayAssets::Note(n.clone()));
				}
			}
		});
		ui.vertical(|ui|{
			ui.heading(&self.locale.default_renote_visibility);
			let old=self.state.default_renote_visibility.clone();
			ui.radio_value(&mut self.state.default_renote_visibility,data_model::Visibility::Public,&self.locale.visibility_public);
			ui.radio_value(&mut self.state.default_renote_visibility,data_model::Visibility::Home,&self.locale.visibility_home);
			ui.radio_value(&mut self.state.default_renote_visibility,data_model::Visibility::Followers,&self.locale.visibility_followers);
			if old!=self.state.default_renote_visibility{
				self.state.write(&self.delay_assets);
			}
		});
	}
	fn media(&self,ui:&mut egui::Ui,lock:&mut Option<ZoomMediaView>){
		fn view<F>(ui:&mut egui::Ui,img:egui::Image<'static>,close:F)where F:FnOnce()->(){
			let img=img.max_width(ui.available_width());
			let img=img.max_height(ui.available_height());
			let img=if let Some((x,y))=img.size().map(|v|(v.x,v.y)){
				img.fit_to_exact_size(egui::Vec2 { x: x*10f32, y: y*10f32 })
			}else{
				img
			};
			let img=egui::Button::image(img);
			let img=img.stroke(egui::Stroke::new(0f32,Color32::from_black_alpha(0u8)));
			let img=img.frame(false);
			ui.horizontal_centered(|ui|{
				if img.ui(ui).clicked(){
					close();
				}
			});
		}
		let v=lock.as_ref().unwrap();
		if let Some(img)=v.original_img.get(self.animate_frame){
			view(ui,img,||{
				lock.take();
			});
			return;
		}else{
			if let Some(img)=&v.preview{
				view(ui,img.clone(),||{
					lock.take();
				});
			}
		}
	}
	fn license(&mut self,ui:&mut egui::Ui,ctx:&egui::Context){
		if ui.button(&self.locale.close_license).clicked(){
			self.view_license=false;
			ctx.request_repaint();
		}
		ScrollArea::vertical().show(ui,|ui|{
			ui.label(include_str!("../include/License.txt"));
		});
	}
}

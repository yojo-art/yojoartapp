#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data_model;
mod load_misskey;
use std::{io::{Read, Write}, sync::Arc};

use eframe::{egui, NativeOptions};

use egui::{Color32, FontData, FontFamily, ScrollArea, Widget};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;
#[cfg(target_os="android")]
mod android_native;
#[cfg(not(target_os="android"))]
pub fn open(){
	env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
	let options = eframe::NativeOptions {
		..Default::default()
	};
	common(options,|_|{});
}
#[derive(Debug,Serialize,Deserialize)]
pub struct ConfigFile{
	token: Option<String>,
	instance:Option<String>,
	is_animation:Option<bool>,
	top:Option<u32>,
}
#[derive(Debug,Serialize,Deserialize)]
pub struct LocaleFile{
	show_nsfw: String,
	show_cw: String,
	renote:String,
	appname:String,
	close_license:String,
	show_license:String,
	websocket:String,
	nsfw_always_show:String,
	open_settings:String,
	close_settings:String,
	load_old_timeline:String,
	auto_old_timeline:String,
}
fn load_config()->(String,Arc<ConfigFile>){
	let config_path=match std::env::var("YAC_CONFIG_PATH"){
		Ok(path)=>{
			if path.is_empty(){
				"config.json".to_owned()
			}else{
				path
			}
		},
		Err(_)=>"config.json".to_owned()
	};
	if !std::path::Path::new(&config_path).exists(){
		let default_config=ConfigFile{
			token:None,
			instance:None,
			is_animation:Some(data_model::DEFAULT_ANIMATION),
			top:Some(0u32),
		};
		let default_config=serde_json::to_string_pretty(&default_config).unwrap();
		std::fs::File::create(&config_path).expect("create default config.json").write_all(default_config.as_bytes()).unwrap();
	}
	let config:ConfigFile=serde_json::from_reader(std::fs::File::open(&config_path).unwrap()).unwrap();
	(config_path,Arc::new(config))
}
fn load_locale()->Arc<LocaleFile>{
	let locale_json=include_str!("locale/ja_jp.json");
	let locale:LocaleFile=serde_json::from_reader(std::io::Cursor::new(locale_json)).unwrap();
	Arc::new(locale)
}
async fn delay_assets(mut recv:Receiver<data_model::DelayAssets>,ctx:egui::Context,client:Client,config:Arc<ConfigFile>){
	//tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
	let mut note_buf=Vec::with_capacity(4);
	let mut job_buf=Vec::with_capacity(4);
	let mut emoji_job_buf=Vec::with_capacity(4);
	loop{
		let limit=note_buf.capacity();
		if recv.recv_many(&mut note_buf,limit).await==0{
			return;
		}
		async fn load_note(note:Arc<data_model::Note>,ctx:&egui::Context,client:&Client,config:&Arc<ConfigFile>){
			let mut job_buf=Vec::with_capacity(4);
			let mut job_buf2=Vec::with_capacity(4);
			let mut job_buf_emojis=Vec::with_capacity(32);
			if let Some(instance)=note.user.instance.clone(){
				if !instance.icon.loaded(){
					let client=client.clone();
					let ctx=ctx.clone();
					let config=config.clone();
					job_buf2.push(async move{
						instance.icon.load(&client).await;
						instance.icon.load_gpu(&ctx,&config).await;
					});
				}
			}
			async fn load_url(emoji:Arc<data_model::UrlImage>,client: reqwest::Client,ctx: egui::Context,config:Arc<ConfigFile>){
				//tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;
				emoji.load(&client).await;
				emoji.load_gpu(&ctx,&config).await;
			}
			for emoji in note.user.display_name.emojis(){
				if !emoji.loaded(){
					job_buf_emojis.push(load_url(emoji.clone(),client.clone(),ctx.clone(),config.clone()));
				}
			}
			for emoji in note.text.emojis(){
				if !emoji.loaded(){
					job_buf_emojis.push(load_url(emoji.clone(),client.clone(),ctx.clone(),config.clone()));
				}
			}
			for emoji in note.reactions.emojis(){
				if !emoji.loaded(){
					job_buf_emojis.push(load_url(emoji.clone(),client.clone(),ctx.clone(),config.clone()));
				}
			}
			for file in note.files.iter(){
				if let Some(urlimg)=file.img.as_ref(){
					if !urlimg.loaded(){
						job_buf_emojis.push(load_url(urlimg.clone(),client.clone(),ctx.clone(),config.clone()));
					}
				}
			}
			if !note.user.icon.loaded(){
				let client=client.clone();
				let ctx=ctx.clone();
				let config=config.clone();
				job_buf.push(async move{
					note.user.icon.load(&client).await;
					note.user.icon.load_gpu(&ctx,&config).await;
				});
			}
			futures::join!(
				futures::future::join_all(job_buf.drain(..)),
				futures::future::join_all(job_buf2.drain(..)),
				futures::future::join_all(job_buf_emojis.drain(..)),
			);
		}
		for a in note_buf.drain(..){
			match a {
				data_model::DelayAssets::Note(note) => {
					let ctx=ctx.clone();
					let client=client.clone();
					let config=config.clone();
					let q=note.quote.clone();
					job_buf.push(async move{
						futures::join!(
							async{
								if let Some(note)=q{
									load_note(note,&ctx,&client,&config).await
								}
							},
							load_note(note,&ctx,&client,&config)
						);
					});
				},
				data_model::DelayAssets::Emoji(cache,emoji) => {
					let ctx=ctx.clone();
					let client=client.clone();
					let config=config.clone();
					emoji_job_buf.push(async move{
						let (id,url)=emoji.to_id_url(&cache);
						println!("load emoji {} \t\t{}",id,&url);
						let emoji=cache.load(emoji.into_id(),&url).await;
						let img=emoji.url_image();
						if !img.loaded(){
							img.load(&client).await;
							img.load_gpu(&ctx,&config).await;
						}
					});
				},
			}
		}
		let d:Vec<_>=job_buf.drain(..).collect();
		let d2:Vec<_>=emoji_job_buf.drain(..).collect();
		let ctx=ctx.clone();
		tokio::runtime::Handle::current().spawn(async move{
			futures::join!(
				futures::future::join_all(d),
				futures::future::join_all(d2),
			);
			ctx.request_repaint();
		});
	}
	//tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;
	//user.icon.unload().await;
}
fn common<F>(options:NativeOptions,ime_show:F)where F:FnMut(&mut bool)+'static{
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
	let config=load_config();
	let locale=load_locale();
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
				let mut gz=flate2::read::GzDecoder::new(std::io::Cursor::new(include_bytes!("NotoSansJP-Medium.ttf.gz")));
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
				let mut gz=flate2::read::GzDecoder::new(std::io::Cursor::new(include_bytes!("themify.ttf.gz")));
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
				rt.block_on(delay_assets(assets_recv,ctx,client0,config0));
			});
			Box::new(MyApp{
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
				now_tl:load_misskey::TimeLine::Home,
				auto_update:false,
				nsfw_always_show:false,
				view_media:std::sync::Mutex::new(None),
				view_license:false,
				view_config:false,
				auto_old_timeline:false,
				view_old_timeline:0f32,
				open_timeline:std::sync::Mutex::new(None),
			})
		}),
	).unwrap();
}
struct MyApp<F>{
	config:(String, Arc<ConfigFile>),
	locale:Arc<LocaleFile>,
	emojis:Option<data_model::EmojiCache>,
	reaction_table:Vec<data_model::LocalEmojis>,
	emojis_recv:Receiver<data_model::EmojiCache>,
	reaction_picker:std::sync::Mutex<Option<String>>,
	input_text:String,
	show_ime:bool,
	button_handle: Box<F>,
	notes:Vec<Arc<data_model::Note>>,
	rcv:Receiver<Arc<data_model::Note>>,
	dummy: data_model::UrlImage,
	animate_frame:u64,
	delay_assets:tokio::sync::mpsc::Sender<data_model::DelayAssets>,
	show_cw:std::sync::Mutex<Option<String>>,
	reload: tokio::sync::mpsc::Sender<load_misskey::LoadSrc>,
	client: Client,
	themify:egui::FontFamily,
	auto_update:bool,
	now_tl:load_misskey::TimeLine,
	nsfw_always_show:bool,
	view_media:std::sync::Mutex<Option<ZoomMediaView>>,
	view_license:bool,
	view_config:bool,
	auto_old_timeline:bool,
	view_old_timeline:f32,
	open_timeline:std::sync::Mutex<Option<(Option<load_misskey::TimeLine>,Option<String>)>>,
}
struct ZoomMediaView{
	original_img:Arc<data_model::UrlImage>,
	preview:Option<egui::Image<'static>>,
}
impl <F> eframe::App for MyApp<F> where F:FnMut(&mut bool)+'static{
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
impl <F> MyApp<F>{
	fn config(&mut self,ui:&mut egui::Ui,ctx:&egui::Context){
		if ui.button(&self.locale.close_settings).clicked(){
			self.view_config=false;
			ctx.request_repaint();
			return;
		}
		ui.checkbox(&mut self.nsfw_always_show,&self.locale.nsfw_always_show);
		if ui.button(&self.locale.show_license).clicked(){
			self.view_license=true;
			ctx.request_repaint();
			return;
		}
		ui.checkbox(&mut self.auto_old_timeline,&self.locale.auto_old_timeline);
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
			ui.label(include_str!("License.txt"));
		});
	}
	fn load(&mut self,tl:Option<load_misskey::TimeLine>,until_id:Option<String>){
		self.view_old_timeline=2f32;
		let reload=self.reload.clone();
		if reload.max_capacity()==reload.capacity(){
			if let Some(tl)=&tl{
				if self.now_tl!=*tl{
					self.notes.clear();
				}
				self.now_tl=tl.clone();
			}
			if until_id.is_some(){
				self.notes.clear();
			}
			let known_notes=self.notes.clone();
			let websocket=self.auto_update;
			let tl=tl.unwrap_or_else(||self.now_tl.clone());
			std::thread::spawn(move||{
				tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
					let _=reload.send(load_misskey::LoadSrc::TimeLine(load_misskey::TLOption{
						until_id,
						limit:30,
						tl,
						known_notes,
						websocket,
					})).await;
				});
			});
		}
	}
	fn timeline(&mut self,ui:&mut egui::Ui,ctx:&egui::Context){
		if let Some((tl,until_id))={
			let mut lock=self.open_timeline.lock().unwrap();
			let v=lock.take();
			v
		}{
			self.load(tl,until_id);
		}
		ui.horizontal_wrapped(|ui|{
			ui.heading(&self.locale.appname);
			if ui.button("HTL").clicked(){
				self.load(Some(load_misskey::TimeLine::Home),None);
			}
			if ui.button("GTL").clicked(){
				self.load(Some(load_misskey::TimeLine::Global),None);
			}
			if ui.checkbox(&mut self.auto_update,&self.locale.websocket).changed(){
				self.load(None,None);
			}
			if self.view_old_timeline>=1f32&&self.view_old_timeline<2f32{
				if let Some(n)=self.notes.first().map(|n|n.id.to_string()){
					self.auto_update=false;
					self.load(None,Some(n));
				}
			}
			if ui.button(&self.locale.open_settings).clicked(){
				self.view_config=true;
				ctx.request_repaint();
				return;
			}
			if !self.rcv.is_empty()||!self.emojis_recv.is_empty(){
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Max),|ui|{
					egui::ProgressBar::new(0f32).desired_width(10f32).animate(true).ui(ui);
				});
			}
		});
		if self.config.1.token.is_none(){
			ui.heading("tokenが指定されていません");
			ui.label(format!("{}を編集してください",self.config.0));
		}
		if self.config.1.instance.is_none(){
			ui.heading("instanceが指定されていません(https://misskey.example.com)");
			ui.label(format!("{}を編集してください",self.config.0));
		}
		ui.text_edit_singleline(&mut self.input_text);
		if let Ok(n)=self.rcv.try_recv(){
			//blurhashは即座に読み込む
			tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
				for f in &n.files{
					if let Some(blurhash)=&f.blurhash{
						blurhash.load_gpu(ctx,&self.config.1).await;
					}
				}
				if let Some(n)=n.quote.as_ref(){
					for f in &n.files{
						if let Some(blurhash)=&f.blurhash{
							blurhash.load_gpu(ctx,&self.config.1).await;
						}
					}
				}
			});
			let mut index=None;
			let mut idx=0;
			for old in &self.notes{
				if old.id==n.id{
					index=Some(idx as usize);
					break;
				}
				idx+=1;
			}
			if let Some(rm)=index{
				//同一ノート内容更新
				self.notes.remove(rm);
				self.notes.insert(rm,n);
			}else{
				self.notes.push(n);
				self.view_old_timeline=0f32;
			}
			if self.notes.len()>30{
				self.notes.remove(0);
			}
		}
//			ui.add(egui::Slider::new(&mut self.age, 0..=120).text("age"));
/*
		if ui.button("Increment").clicked() {
			self.age += 1;
			(self.button_handle)(&mut self.show_ime);
		}
*/
		let scroll=ScrollArea::vertical().show(ui,|ui|{
			let width=ui.available_width();
			for note in self.notes.iter().rev(){
				ui.allocate_ui([width,0f32].into(),|ui|{
					self.note_ui(ui,note);
				});
			}
			if !self.auto_old_timeline{
				if egui::Button::new(&self.locale.load_old_timeline).ui(ui).clicked(){
					self.view_old_timeline=1f32;
				};
			}else{
				egui::ProgressBar::new(self.view_old_timeline).ui(ui);
			}
		});
		if self.auto_old_timeline{
			if self.view_old_timeline>=2f32{
				//now loading
			}else if scroll.state.offset.y==0f32.max(scroll.content_size.y-scroll.inner_rect.height()){
				if self.view_old_timeline<1f32{
					self.view_old_timeline+=0.01f32;
				}
			}else if self.view_old_timeline>0f32{
				self.view_old_timeline-=0.01f32;
			}
		}
	}
	fn time_label(&self,ui:&mut egui::Ui,note:&data_model::Note){
		let label=if note.visibility!=data_model::Visibility::Public{
			let s=match note.visibility {
				data_model::Visibility::Public => unimplemented!(),
				data_model::Visibility::Home => "\u{e69b}",
				data_model::Visibility::Followers => "\u{e62b}",
				data_model::Visibility::Specified => "\u{e75a}",
			};
			note.created_at_label()+s
		}else{
			note.created_at_label()
		};
		let font=egui::FontId::new(13.0,self.themify.clone());
		egui::Label::new(egui::RichText::new(&label).font(font.clone()).color(Color32::from_black_alpha(0))).wrap(false).ui(ui);
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Max),|ui|{
			egui::Label::new(egui::RichText::new(label).font(font)).wrap(false).ui(ui).on_hover_text(format!("{:?}",note.visibility));
		});
	}
	fn normal_note(&self,ui:&mut egui::Ui,note:&Arc<data_model::Note>,quote:Option<&Arc<data_model::Note>>,top_level:bool){
		let user=&note.user;
		ui.horizontal_top(|ui| {
			//表示部分
			//ユーザーアイコン60x60
			let icon=if top_level{
				let icon=self.get_image(&user.icon);
				let icon=icon.max_size([60f32,60f32].into());
				let icon=icon.rounding(egui::Rounding::from(30f32));
				icon
			}else{
				let icon=self.get_image(&user.icon);
				let icon=icon.max_size([20f32,20f32].into());
				let icon=icon.rounding(egui::Rounding::from(10f32));
				icon
			};
			let icon=egui::Button::image(icon);
			let icon=icon.fill(Color32::from_black_alpha(0));
			if icon.ui(ui).clicked(){
				self.open_timeline.lock().unwrap().replace((Some(load_misskey::TimeLine::User(note.user.id.clone())),None));
			}
			if top_level{
				//右に10
				ui.add_space(10f32);
			}
			ui.vertical(|ui|{
				//ユーザー名
				ui.horizontal_wrapped(|ui|{
					ui.spacing_mut().item_spacing=[0f32,0f32].into();
					user.display_name.render(ui,true,&self.dummy,self.animate_frame);
					ui.label(format!("@{}",user.username));
					if let Some(instance)=user.instance.as_ref(){
						ui.colored_label(Color32::from_gray(100),format!("@{}",instance.host()));
					}
					//時刻と可視性
					self.time_label(ui,note);
				});
				//インスタンス情報
				if let Some(instance)=user.instance.as_ref(){
					ui.horizontal(|ui|{
						let icon=self.get_image(&instance.icon);
						let icon=icon.max_size([15f32,15f32].into());
						icon.ui(ui);
						let mut ui=ui.child_ui(ui.available_rect_before_wrap(),egui::Layout::default());
						let mut job = egui::text::LayoutJob::single_section(
							instance.display_name().to_owned(),
							egui::TextFormat {
								color:Color32::from_gray(255),
								background:instance.theme_color(),
								font_id:egui::FontId{
									size: 10f32,
									..Default::default()
								},
								..Default::default()
							},
						);
						job.wrap = egui::text::TextWrapping {
							max_rows:1,
							break_anywhere: true,
							overflow_character: Some('…'),
							..Default::default()
						};
						ui.label(job);
						//Label::new(egui::RichText::new(instance.display_name()).color(Color32::from_gray(255)).background_color(instance.theme_color())).ui(ui);
					});
				}
				//トップレベル要素ならCWか確認する
				let show_note=if !top_level{
					true
				}else if let Some(cw)=note.cw.as_ref(){
					let mut show_cw=self.show_cw.lock().unwrap();
					let mut checked=Some(note.id.as_str())==show_cw.as_ref().map(|s|s.as_str());
					cw.render(ui,false,&self.dummy,self.animate_frame);
					if ui.checkbox(&mut checked,self.locale.show_cw.as_str()).changed(){
						*show_cw=if checked{
							Some(note.id.to_owned())
						}else{
							None
						}
					}
					checked
				}else{
					true
				};
				//CWが展開されているか子要素
				if show_note{
					note.text.render(ui,false,&self.dummy,self.animate_frame);
					//添付ファイル
					let width=ui.available_width();
					for file in &note.files{
						let show_sensitive=file.show_sensitive.load(std::sync::atomic::Ordering::Relaxed);
						let show_sensitive=!file.is_sensitive||show_sensitive||self.nsfw_always_show;
						let img_opt=if !show_sensitive{
							None
						}else{
							file.image(self.animate_frame).map(|v|Some(v))
						};
						if let Some(img)=img_opt.unwrap_or_else(||{
							file.blurhash.as_ref().map(|img|img.get(self.animate_frame)).unwrap_or_default()
						}){
							//利用できる画像
							let img=img.fit_to_exact_size((width,f32::MAX).into());
							let img=img.max_width(width);
							let img=egui::Button::image(img);
							//let img=img.rounding(egui::Rounding::default());
							let img=img.stroke(egui::Stroke::new(0f32,Color32::from_black_alpha(0u8)));
							let img=img.frame(false);
							let res=img.ui(ui);
							if res.clicked(){
								if !show_sensitive{
									file.show_sensitive.store(true,std::sync::atomic::Ordering::Relaxed);
								}else if let Some(url)=file.original_url.clone(){
									if let Ok(mut lock)=self.view_media.lock(){
										let preview=file.image(self.animate_frame).map(|v|Some(v)).unwrap_or_else(||{
											file.blurhash.as_ref().map(|img|img.get(self.animate_frame)).unwrap_or_default()
										});
										let original_img=Arc::new(data_model::UrlImage::from(url));
										let original_img0=original_img.clone();
										let ctx=ui.ctx().clone();
										let client=self.client.clone();
										let config=self.config.1.clone();
										std::thread::spawn(move||{
											tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async{
												//tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
												original_img0.load(&client).await;
												original_img0.load_gpu(&ctx,&config).await;
											});
											ctx.request_repaint();
										});
										*lock=Some(ZoomMediaView{
											original_img,
											preview,
										});
									}
								}
								println!("ZOOM {:?}",file.original_url);
							}
							if !show_sensitive{
								let mut ui=ui.child_ui(res.rect,egui::Layout::top_down(egui::Align::Center));
								let width=ui.available_width();
								let height=ui.available_height();
								let job = egui::text::LayoutJob::single_section(
									self.locale.show_nsfw.clone(),
									egui::TextFormat {
										color:Color32::from_gray(0),
										font_id:egui::FontId{
											size: 30f32,
											..Default::default()
										},
										..Default::default()
									},
								);
								let bt=egui::Button::new(job);
								let bt=bt.min_size([width,height].into());
								let bt=bt.frame(false);
								let bt=bt.fill(Color32::from_black_alpha(0));
								let bt=bt.stroke(egui::Stroke::new(0f32,Color32::from_black_alpha(0u8)));
								if bt.ui(&mut ui).clicked(){
									file.show_sensitive.store(true,std::sync::atomic::Ordering::Relaxed);
								}
							}
						}else if file.is_image(){
							//利用できない画像
							let img=self.dummy.get(self.animate_frame).unwrap().max_width(width);
							img.ui(ui);
						}
					}
					//引用
					if let Some(quote)=quote{
						self.normal_note(ui,quote,None,false);
					}
				}
				ui.horizontal_wrapped(|ui|{
					for (emoji,count) in note.reactions.emojis.iter(){
						let id=emoji.id_raw().id();
						let img=emoji.image(self.animate_frame).unwrap_or_else(||self.dummy.get(self.animate_frame).unwrap());
						let img=img.max_height(20f32);
						let img=egui::widgets::Button::image_and_text(img, format!("{}",count));
						let img=if id.contains("@"){
							let img=img.frame(false);
							let img=img.fill(Color32::from_black_alpha(0u8));
							img.stroke(egui::Stroke::new(0f32,Color32::from_black_alpha(0u8)))
						}else{
							img
						};
						//ui.add_enabled(false,img).on_hover_text(emoji.id());
						if img.ui(ui).on_hover_text(id.as_str()).clicked(){
							tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
								let _=self.delay_assets.send(data_model::DelayAssets::Note(note.clone())).await;
							});
							if let Some(emojis)=self.emojis.as_ref(){
								let e=data_model::LocalEmojis::from_id(emoji.id_raw().to_owned(),emojis);
								if let Some(e)=e{
									self.reaction_send(note,&e);
								}
							}
						}
					}
				});
			});
		});
	}
	fn note_ui(&self,ui:&mut egui::Ui,note:&Arc<data_model::Note>){
		if let Some(quote)=note.quote.as_ref(){
			if !note.text.is_empty(){
				self.normal_note(ui,note,Some(quote),true);
			}else{
				ui.horizontal_wrapped(|ui|{
					let icon=self.get_image(&note.user.icon);
					//ユーザーアイコン20x20
					let icon=icon.max_size([20f32,20f32].into());
					let icon=icon.rounding(egui::Rounding::from(10f32));
					let icon=egui::Button::image(icon);
					let icon=icon.fill(Color32::from_black_alpha(0));
					if icon.ui(ui).clicked(){
						self.open_timeline.lock().unwrap().replace((Some(load_misskey::TimeLine::User(note.user.id.clone())),None));
					}
					note.user.display_name.render(ui,true,&self.dummy,self.animate_frame);
					ui.label(self.locale.renote.as_str());
					//時刻と可視性
					self.time_label(ui,note);
				});
				self.normal_note(ui,quote,None,true);
			}
		}else{
			self.normal_note(ui,note,None,true);
		}
		if ui.button("Reaction").clicked(){
			let mut lock=self.reaction_picker.lock().unwrap();
			if lock.as_ref().map(|id|id==&note.id).unwrap_or_default(){
				*lock=None;
			}else{
				*lock=Some(note.id.clone());
			}
		}
		if let Some(id)=self.reaction_picker.lock().unwrap().as_ref(){
			if id==&note.id{
				if let Some(emojis)=&self.emojis{
					self.reaction_picker(ui,emojis,&note);
				}
			}
		}
		//セパレーター
		ui.add_space(5f32);
		ui.separator();
		ui.add_space(5f32);
	}
	fn get_image(&self,icon:&data_model::UrlImage)->egui::Image<'static>{
		icon.get(self.animate_frame).unwrap_or_else(||self.dummy.get(self.animate_frame).unwrap())
	}
	fn reaction_send(&self,note:&data_model::Note,emoji:&data_model::LocalEmojis)->bool{
		let build=self.client.post(format!("{}/api/notes/reactions/create",self.config.1.instance.as_ref().unwrap()));
		let build=build.header(reqwest::header::CONTENT_TYPE,"application/json");
		#[derive(Debug,Serialize,Deserialize)]
		struct ReactionCreatepayload{
			#[serde(rename = "noteId")]
			note_id:String,
			reaction:String,
			i:String,
		}
		let payload=ReactionCreatepayload{
			note_id:note.id.clone(),
			reaction:emoji.reaction(),
			i:self.config.1.token.as_ref().unwrap().clone(),
		};
		println!("リアクション送信 {:?}",payload);
		let build=build.body(serde_json::to_string(&payload).unwrap());
		let ok=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async{
			let res=build.send().await;
			match res{
				Ok(res)=>{
					let status=res.status().as_u16();
					println!("ReactionSendStatus {}",status);
					let ok=status==204;
					if ok{
						let _=self.reload.send(load_misskey::LoadSrc::Note(note.id.clone())).await;
					}
					ok
				},
				Err(e)=>{
					eprintln!("{:?}",e);
					false
				}
			}
		});
		ok
	}
	fn reaction_picker(&self,ui:&mut egui::Ui,emojis:&data_model::EmojiCache,note:&data_model::Note){
		let emoji_size=25f32;
		let width=ui.available_width();
		let horizontal_count=(width/(emoji_size+15f32)-0.5).round() as usize;
		let row_height=emoji_size+8f32;
		//ui.label(format!("{}/{}",width,horizontal_count));
		let rows=self.reaction_table.len()/horizontal_count+{
			if self.reaction_table.len()%horizontal_count==0{
				0
			}else{
				1
			}
		};
		ui.allocate_ui([width,5f32*row_height].into(),|ui|{
			ScrollArea::vertical()
				.max_height(5f32*row_height)
				.auto_shrink(false)
				.id_source(&note.id)
				.show_rows(ui,row_height,rows,|ui,row_range|{
				for row in row_range{
					let start=row*horizontal_count;
					let end=start+horizontal_count;
					let s=&self.reaction_table[start..end.min(self.reaction_table.len())];
					ui.horizontal(|ui|{
						for e in s{
							let id=e.to_id_string().to_string();
							let img=tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
								let emoji=emojis.get(e.clone().into_id()).await;
								if let Some(emoji)=emoji{
									Some(emoji)
								}else{
									let _=self.delay_assets.send(data_model::DelayAssets::Emoji(emojis.clone(),e.clone())).await;
									None
								}
							});
							let img=match img.map(|img|img.get(self.animate_frame)).unwrap_or_default(){
								Some(img)=>img,
								None=>self.dummy.get(self.animate_frame).unwrap()
							};
							let img=img.fit_to_exact_size([f32::MAX,f32::MAX].into());
							let img=img.max_width(emoji_size);
							let img=img.max_height(emoji_size);
							let bt=egui::Button::image(img).min_size([10f32,row_height].into());
							let bt=bt.min_size([emoji_size+8.0,emoji_size+8.0].into());
							if bt.ui(ui).on_hover_text(&id).clicked(){
								self.reaction_send(note,e);
							}
						}
					});
				}
			});
		});
	}
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data_model;
mod load_misskey;
use std::{borrow::{Borrow, Cow}, io::{Read, Write}, sync::Arc};

use eframe::{egui, NativeOptions};

use egui::{Color32, FontData, FontFamily, Label, ScrollArea, Widget};
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
}
fn load_config()->(String,Arc<ConfigFile>,Arc<LocaleFile>){
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
	let locale_json=include_str!("locale/ja_jp.json");
	let locale:LocaleFile=serde_json::from_reader(std::io::Cursor::new(locale_json)).unwrap();
	(config_path,Arc::new(config),Arc::new(locale))
}
async fn delay_assets(mut recv:Receiver<Arc<data_model::Note>>,ctx:egui::Context,client:Client,config:Arc<ConfigFile>){
	//tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
	let mut note_buf=Vec::with_capacity(4);
	let mut job_buf=Vec::with_capacity(4);
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
		for note in note_buf.drain(..){
			if let Some(note)=note.quote.clone(){
				job_buf.push(load_note(note,&ctx,&client,&config));
			}
			job_buf.push(load_note(note,&ctx,&client,&config));
		}
		futures::future::join_all(job_buf.drain(..)).await;
		ctx.request_repaint();
	}
	//tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;
	//user.icon.unload().await;
}
fn common<F>(options:NativeOptions,ime_show:F)where F:FnMut(&mut bool)+'static{
	let config=load_config();
	let (assets,assets_recv)=tokio::sync::mpsc::channel(10);
	let (note_ui,recv)=tokio::sync::mpsc::channel(4);
	let (reload,reload_recv)=tokio::sync::mpsc::channel(1);
	let config0=config.1.clone();
	let client=Client::new();
	let client0=client.clone();
	let assets0=assets.clone();
	std::thread::spawn(move||{
		let rt=tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
		rt.block_on(load_misskey::load_misskey(config0,note_ui,assets0,client0,reload_recv))
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
				input_text:String::new(),
				show_ime:false,
				button_handle:Box::new(ime_show),
				notes:vec![],
				rcv:recv,
				dummy,
				animate_frame:0u64,
				delay_assets:assets,
				show_cw:std::sync::Mutex::new(None),
				reload,
				media_view:std::sync::Mutex::new(None),
				client,
				themify,
				view_license:false,
				now_tl:load_misskey::TimeLine::Home,
				auto_update:false,
				nsfw_always_show:false,
			})
		}),
	).unwrap();
}
struct MyApp<F>{
	config:(String, Arc<ConfigFile>,Arc<LocaleFile>),
	input_text:String,
	show_ime:bool,
	button_handle: Box<F>,
	notes:Vec<Arc<data_model::Note>>,
	rcv:Receiver<Arc<data_model::Note>>,
	dummy: data_model::UrlImage,
	animate_frame:u64,
	delay_assets:tokio::sync::mpsc::Sender<Arc<data_model::Note>>,
	show_cw:std::sync::Mutex<Option<String>>,
	reload: tokio::sync::mpsc::Sender<load_misskey::TLOption>,
	media_view:std::sync::Mutex<Option<ZoomMediaView>>,
	client: Client,
	themify:egui::FontFamily,
	view_license:bool,
	auto_update:bool,
	now_tl:load_misskey::TimeLine,
	nsfw_always_show:bool,
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
		egui::CentralPanel::default().show(ctx, |ui| {
			if let Ok(mut lock)=self.media_view.lock(){
				if let Some(v)=lock.as_ref(){
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
						return;
					}
				}
			}
			ui.add_space(self.config.1.top.unwrap_or(0) as f32);
			if self.view_license{
				if ui.button(&self.config.2.close_license).clicked(){
					self.view_license=false;
					ctx.request_repaint();
					return;
				}
				ScrollArea::vertical().show(ui,|ui|{
					ui.label(include_str!("License.txt"));
				});
				return;
			}
			ui.horizontal_wrapped(|ui|{
				ui.heading(&self.config.2.appname);
				fn load<F>(app:&mut MyApp<F>,limit:u8,tl:load_misskey::TimeLine){
					let reload=app.reload.clone();
					if reload.max_capacity()==reload.capacity(){
						if app.now_tl!=tl{
							app.notes.clear();
						}
						app.now_tl=tl;
						let known_notes=app.notes.clone();
						let websocket=app.auto_update;
						std::thread::spawn(move||{
							tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
								let _=reload.send(load_misskey::TLOption{
									limit,
									tl,
									known_notes,
									websocket,
								}).await;
							});
						});
					}
				}
				if ui.button("HTL").clicked(){
					load(self,30,load_misskey::TimeLine::Home);
				}
				if ui.button("GTL").clicked(){
					load(self,30,load_misskey::TimeLine::Global);
				}
				if ui.checkbox(&mut self.auto_update,&self.config.2.websocket).changed(){
					load(self,30,self.now_tl);
				}
				ui.checkbox(&mut self.nsfw_always_show,&self.config.2.nsfw_always_show);
				/*
				if ui.button("キャッシュ削除").clicked(){
					let _=std::fs::remove_dir_all(data_model::cache_dir());
				}
				*/
				if ui.button(&self.config.2.show_license).clicked(){
					self.view_license=true;
					ctx.request_repaint();
					return;
				}
				if !self.rcv.is_empty(){
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
			ScrollArea::vertical().show(ui,|ui|{
				let width=ui.available_width();
				for note in self.notes.iter().rev(){
					ui.allocate_ui([width,0f32].into(),|ui|{
						self.note_ui(ui,note);
					});
				}
			});
		});
	}
}
impl <F> MyApp<F>{
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
			if top_level{
				let icon=self.get_image(&user.icon);
				let icon=icon.max_size([60f32,60f32].into());
				let icon=icon.rounding(egui::Rounding::from(30f32));
				icon.ui(ui);
				//右に10
				ui.add_space(10f32);
			}else{
				let icon=self.get_image(&user.icon);
				let icon=icon.max_size([20f32,20f32].into());
				let icon=icon.rounding(egui::Rounding::from(10f32));
				icon.ui(ui);
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
					if ui.checkbox(&mut checked,self.config.2.show_cw.as_str()).changed(){
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
									if let Ok(mut lock)=self.media_view.lock(){
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
									self.config.2.show_nsfw.clone(),
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
						let img=emoji.image(self.animate_frame).unwrap_or_else(||self.dummy.get(self.animate_frame).unwrap());
						let img=img.max_height(20f32);
						let img=egui::widgets::Button::image_and_text(img, format!("{}",count));
						let img=if emoji.id().contains("@"){
							let img=img.frame(false);
							let img=img.fill(Color32::from_black_alpha(0u8));
							img.stroke(egui::Stroke::new(0f32,Color32::from_black_alpha(0u8)))
						}else{
							img
						};
						//ui.add_enabled(false,img).on_hover_text(emoji.id());
						if img.ui(ui).on_hover_text(emoji.id()).clicked(){
							tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async{
								let _=self.delay_assets.send(note.clone()).await;
							});
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
					icon.ui(ui);
					note.user.display_name.render(ui,true,&self.dummy,self.animate_frame);
					ui.label(self.config.2.renote.as_str());
					//時刻と可視性
					self.time_label(ui,note);
				});
				self.normal_note(ui,quote,None,true);
			}
		}else{
			self.normal_note(ui,note,None,true);
		}
		//セパレーター
		ui.add_space(5f32);
		ui.separator();
		ui.add_space(5f32);
	}
	fn get_image(&self,icon:&data_model::UrlImage)->egui::Image<'static>{
		icon.get(self.animate_frame).unwrap_or_else(||self.dummy.get(self.animate_frame).unwrap())
	}
}

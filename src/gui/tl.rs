use std::sync::Arc;

use egui::{Color32, ScrollArea, Widget};

use crate::{data_model::{self, DelayAssets, LocalEmojis, Note, Summaly, UrlImage}, gui::utils::ZoomMediaView, load_misskey::{self, TimeLine}};

use super::main_ui::MainUI;

impl <F> MainUI<F>{
	fn load(&mut self,tl:Option<TimeLine>,until_id:Option<String>){
		self.view_old_timeline=2f32;
		let reload=self.reload.clone();
		if reload.max_capacity()==reload.capacity(){
			self.state.until_id=until_id.clone();
			if let Some(tl)=&tl{
				if self.state.timeline!=*tl{
					self.state.timeline=tl.clone();
					self.notes.clear();
					if until_id.is_none(){
						self.state.write(&self.delay_assets);
					}
				}
			}
			if until_id.is_some(){
				self.notes.clear();
				self.state.write(&self.delay_assets);
			}
			let known_notes=self.notes.clone();
			let websocket=self.auto_update;
			let tl=tl.unwrap_or_else(||self.state.timeline.clone());
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
	pub(super) fn timeline(&mut self,ui:&mut egui::Ui,ctx:&egui::Context){
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
	/*
		if ui.button("showIME").clicked() {
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
			if !self.state.auto_old_timeline{
				if egui::Button::new(&self.locale.load_old_timeline).ui(ui).clicked(){
					self.view_old_timeline=1f32;
				};
			}else{
				egui::ProgressBar::new(self.view_old_timeline).ui(ui);
			}
		});
		if self.state.auto_old_timeline{
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
	fn url_summaly(&self,url:&str,summaly:&Arc<tokio::sync::Mutex<Option<Summaly>>>,ui:&mut egui::Ui){
		let mut sub_ui=ui.child_ui_with_id_source(ui.available_rect_before_wrap(),egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),url);
		let sub_ui=&mut sub_ui;
		let lock=summaly.blocking_lock();
		let mut title=self.locale.summaly_default_title.as_str();
		let mut description=self.locale.summaly_default_description.as_str();
		let mut sitename=self.locale.summaly_default_sitename.as_str();
		let mut favicon=None;
		let mut thumbnail=None;
		//画面幅を取得しておく
		let available_width=ui.available_width();
		if let Some(a)=lock.as_ref(){
			title=a.title.as_ref().as_ref().map(|s|s.as_str()).unwrap_or(url);
			description=a.description.as_ref().map(|s|{
				let len=s.char_indices().nth(50).map(|(v,_)|v).unwrap_or(s.len());
				&s[..len]
			}).unwrap_or("");
			sitename=a.sitename.as_ref().map(|s|s.as_str()).unwrap_or(url);
			favicon=a.icon.as_ref();
			thumbnail=a.thumbnail.as_ref();
		}
		let long_mode=thumbnail.as_ref().map(|img|img.size().map(|v|v[0] as f32>v[1] as f32*1.3f32).unwrap_or(false)).unwrap_or(false);
		if !long_mode{//正方形or縦長モード
			if let Some(thumbnail)=thumbnail{
				//幅は画面幅の25%上限
				let max_width=available_width*0.25f32;
				//高さは幅の150%上限
				let max_height=max_width*1.5;
				self.get_image(&thumbnail).max_size([max_width,max_height].into()).ui(sub_ui);
			}
		}
		sub_ui.vertical(|sub_ui|{
			sub_ui.strong(title);
			sub_ui.label(description);
			sub_ui.horizontal_wrapped(|sub_ui|{
				if let Some(favicon)=favicon{
					self.get_image(&favicon).max_size([10f32,10f32].into()).ui(sub_ui);
				}
				sub_ui.label(sitename);
			});
			if long_mode{//横長モード
				if let Some(thumbnail)=thumbnail{
					//高さは無制限(暫定的)
					self.get_image(&thumbnail).max_size([available_width,f32::MAX].into()).ui(sub_ui);
				}
			}
		});
		let mut size=sub_ui.min_size();
		size.x=ui.available_width();
		if egui::Button::new("").min_size(size).fill(Color32::from_black_alpha(0)).stroke(egui::Stroke::new(1f32,egui::Color32::from_gray(200))).ui(ui).clicked(){
			//プレビュー拡大
			if let Some(thumbnail)=thumbnail{
				if let Ok(mut lock)=self.view_media.lock(){
					*lock=Some(ZoomMediaView{
						original_img:thumbnail.clone(),
						preview:None,
					});
				}
			}
		}
	}
	fn note_ui(&self,ui:&mut egui::Ui,note:&Arc<Note>){
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
		ui.horizontal_wrapped(|ui|{
			//ノート操作メニュー
			if ui.button(&self.locale.add_reaction).clicked(){
				let mut lock=self.reaction_picker.lock().unwrap();
				if lock.as_ref().map(|id|id==&note.id).unwrap_or_default(){
					*lock=None;
				}else{
					*lock=Some(note.id.clone());
				}
			}
			if ui.button(&self.locale.reload).clicked(){
				let _=self.reload.blocking_send(load_misskey::LoadSrc::Note(note.id.clone()));
			}
			if ui.button(&self.locale.open_in_browser).clicked(){
				ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{}/notes/{}",self.config.1.instance.as_ref().unwrap(),&note.id)));
			}
		});
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
	fn normal_note(&self,ui:&mut egui::Ui,note:&Arc<Note>,quote:Option<&Arc<Note>>,top_level:bool){
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
					let icon=self.get_image(&instance.icon);
					let icon=icon.max_size([15f32,15f32].into());
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
					let mut sub_ui=ui.child_ui(ui.available_rect_before_wrap(),egui::Layout::default());
					let bt=egui::Button::image_and_text(icon,job);
					let bt=bt.rounding(egui::Rounding::same(5f32));
					let bt=bt.frame(false);
					let bt=bt.ui(&mut sub_ui);
					//Label::new(egui::RichText::new(instance.display_name()).color(Color32::from_gray(255)).background_color(instance.theme_color())).ui(ui);
					ui.add_space(bt.rect.height());
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
						let show_sensitive=!file.is_sensitive||show_sensitive||self.state.nsfw_always_show;
						let img_opt=if !show_sensitive{
							None
						}else{
							if self.state.file_thumbnail_mode==crate::FileThumbnailMode::Original{
								match file.original_img.as_ref(){
									Some(v) => {
										v.get(self.animate_frame).map(|v|Some(Some(v))).unwrap_or_default()
									},
									None => None,
								}
							}else{
								file.image(self.animate_frame).map(|v|Some(v))
							}
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
								}else if let Some(original_img)=file.original_img.clone(){
									if let Ok(mut lock)=self.view_media.lock(){
										let preview=file.image(self.animate_frame).map(|v|Some(v)).unwrap_or_else(||{
											file.blurhash.as_ref().map(|img|img.get(self.animate_frame)).unwrap_or_default()
										});
										if !original_img.loaded(){
											let _=self.delay_assets.blocking_send(data_model::DelayAssets::Image(original_img.clone()));
										}
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
					//URLサマリは本文外に描画
					for (url,summaly) in note.text.urls(){
						self.url_summaly(url,summaly,ui);
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
								let _=self.delay_assets.send(DelayAssets::Note(note.clone())).await;
							});
							if let Some(emojis)=self.emojis.as_ref(){
								let e=LocalEmojis::from_id(emoji.id_raw().to_owned(),emojis);
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
}
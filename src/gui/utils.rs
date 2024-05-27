use std::sync::Arc;

use egui::{Color32, ScrollArea, Widget};
use serde::{Deserialize, Serialize};

use crate::{data_model::{DelayAssets, EmojiCache, LocalEmojis, Note, UrlImage, Visibility}, load_misskey};

use super::main_ui::MainUI;

pub(super) struct ZoomMediaView{
	pub(super) original_img:Arc<UrlImage>,
	pub(super) preview:Option<egui::Image<'static>>,
}
impl <F> MainUI<F>{
	pub(super) fn time_label(&self,ui:&mut egui::Ui,note:&Note){
		let label=if note.visibility!=Visibility::Public{
			let s=match note.visibility {
				Visibility::Public => unimplemented!(),
				Visibility::Home => "\u{e69b}",
				Visibility::Followers => "\u{e62b}",
				Visibility::Specified => "\u{e75a}",
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
	pub(super) fn get_image(&self,icon:&UrlImage)->egui::Image<'static>{
		icon.get(self.animate_frame).unwrap_or_else(||self.dummy.get(self.animate_frame).unwrap())
	}
	pub(super) fn renote_send(&self,note:&Note,visibility:Visibility)->bool{
		let build=self.client.post(format!("{}/api/notes/create",self.config.1.instance.as_ref().unwrap()));
		let build=build.header(reqwest::header::CONTENT_TYPE,"application/json");
		#[derive(Debug,Serialize,Deserialize)]
		struct RenoteCreatePayload{
			#[serde(rename = "renoteId")]
			renote_id:String,
			visibility:String,
			i:String,
			#[serde(rename = "localOnly")]
			local_only:bool,
		}
		let id=if note.text.raw.is_empty(){
			match note.quote.as_ref(){
				Some(n)=>n.id.clone(),
				None=>return false,
			}
		}else{
			note.id.clone()
		};
		let payload=RenoteCreatePayload{
			renote_id:id,
			visibility:visibility.to_string(),
			i:self.config.1.token.as_ref().unwrap().clone(),
			local_only:false,
		};
		println!("RN {:?}",payload);
		let build=build.body(serde_json::to_string(&payload).unwrap());
		let ok=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async{
			let res=build.send().await;
			match res{
				Ok(res)=>{
					let status=res.status().as_u16();
					println!("RenoteSendStatus {}",status);
					status==200
				},
				Err(e)=>{
					eprintln!("{:?}",e);
					false
				}
			}
		});
		ok
	}
	pub(super) fn reaction_send(&self,note:&Note,emoji:&LocalEmojis)->bool{
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
	pub(super) fn reaction_picker(&self,ui:&mut egui::Ui,emojis:&EmojiCache,note:&Note){
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
									let _=self.delay_assets.send(DelayAssets::Emoji(emojis.clone(),e.clone())).await;
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

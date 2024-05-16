
use std::{collections::HashMap, sync::Arc};

use chrono::{TimeZone, Utc};
use futures::{StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{data_model::{self, EmojiCache, NoteFile}, ConfigFile};

pub async fn load_misskey(config:Arc<ConfigFile>,note_ui:Sender<Arc<data_model::Note>>,delay_assets:Sender<Arc<data_model::Note>>,client:Client,mut reload_event:Receiver<(u8,TimeLine)>){
	let mut summaly_proxy="https://summaly.yojo.tokyo".to_owned();
	let mut media_proxy="https://proxy.yojo.tokyo".to_owned();
	let mut local_instance="https://misskey.kzkr.xyz";// /api /twemoji /avatar
	if config.token.as_ref().is_none(){
		let mes=format!("token が指定されていません");
		if let Err(e)=note_ui.send(Arc::new(data_model::Note::system_message(mes,"").await)).await{
			eprintln!("{:?}",e);
		}
		return;
	}
	if config.instance.as_ref().is_none(){
		let mes=format!("instance が指定されていません");
		if let Err(e)=note_ui.send(Arc::new(data_model::Note::system_message(mes,"").await)).await{
			eprintln!("{:?}",e);
		}
		return;
	}
	local_instance=config.instance.as_ref().unwrap();
	let meta=meta(&client,local_instance).await;
	if let Err(e)=meta{
		let mes=format!("get api/meta error {}",e);
		if let Err(e)=note_ui.send(Arc::new(data_model::Note::system_message(mes,"").await)).await{
			eprintln!("{:?}",e);
		}
		return;
	}
	let meta=meta.unwrap();
	//media_proxy=meta.media_proxy;
	println!("media_proxy:{}",media_proxy);
	let mut local_emojis=HashMap::new();
	if let Ok(emojis)=emojis(&client,local_instance).await{
		for emoji in emojis.emojis{
			local_emojis.insert(emoji.name,emoji.url);
		}
	}
	println!("{} local emojis",local_emojis.len());
	let emoji_cache=data_model::EmojiCache::new(media_proxy,local_instance,Arc::new(local_emojis));
	let mut instance_cache=HashMap::new();
	let mut user_cache=HashMap::new();
	let mut file_cache=HashMap::new();
	let mut note_cache: HashMap<String, Arc<data_model::Note>>=HashMap::new();
	while let Some(limit) = reload_event.recv().await{
		let htl=read_timeline(&client,local_instance,config.token.as_ref().unwrap().clone(),limit.clone()).await;
		if let Err(e)=htl{
			let mes=format!("get api/notes/{} error {}",limit.1.to_string(),e);
			if let Err(e)=note_ui.send(Arc::new(data_model::Note::system_message(mes,"").await)).await{
				eprintln!("{:?}",e);
			}
			return;
		}
		let notes=htl.unwrap();
		println!("{} notes get",notes.len());
		for note in notes {
			if let Some(n)=note_cache.get(&note.id){
				if let Err(e)=note_ui.send(n.clone()).await{
					eprintln!("{:?}",e);
				}
			}else{
				let n=load_note(note,&mut note_cache,&mut user_cache,&mut instance_cache,&mut file_cache,local_instance,&emoji_cache).await;
				if let Some(n) = n {
					let n=Arc::new(n);
					//println!("@{}: {}", note.user.username, text);
					note_cache.insert(n.id.to_owned(),n.clone());
					let note=n.clone();
					let note_ui=note_ui.clone();
					//tokio::runtime::Handle::current().spawn(async move{
						if let Err(e)=note_ui.send(note).await{
							eprintln!("{:?}",e);
						}
					//});
					let delay_assets=delay_assets.clone();
					tokio::runtime::Handle::current().spawn(async move{
						if let Err(e)=delay_assets.send(n).await{
							eprintln!("{:?}",e);
						}
					});
				}
			};
		}
		let rc=2;//削除しきい値
		let mut remove_targets=vec![];
		for (k,v) in note_cache.iter(){
			let count=Arc::strong_count(v);
			if count<rc{
				remove_targets.push(k.clone());
			}
		}
		for r in &remove_targets{
			note_cache.remove(r);
		}
		let removed_note=remove_targets.len();
		remove_targets.clear();
		for (k,v) in note_cache.iter(){
			let count=Arc::strong_count(v);
			if count<rc{
				remove_targets.push(k.clone());
			}
		}
		for r in &remove_targets{
			note_cache.remove(r);
		}
		println!("note cache\t removed {}({} cached)",removed_note+remove_targets.len(),note_cache.len());
		remove_targets.clear();
		emoji_cache.trim(rc).await;
		for (k,v) in file_cache.iter(){
			if v.is_image(){
				if v.img.as_ref().unwrap().loaded(){
					let count=Arc::strong_count(v.img.as_ref().unwrap());
					//画像は参照が一定に満たない場合
					if count<rc{
						remove_targets.push(k.clone());
					}
				}
			}else{
				//画像ではないメディアは無条件でアンロード
				remove_targets.push(k.clone());
			}
		}
		for r in &remove_targets{
			file_cache.remove(r);
		}
		println!("file_meta cache\t removed {}({} cached)",remove_targets.len(),file_cache.len());
		remove_targets.clear();
		for (k,v) in user_cache.iter(){
			let count=Arc::strong_count(v);
			if count<rc{
				remove_targets.push(k.clone());
			}
		}
		for r in &remove_targets{
			user_cache.remove(r);
		}
		println!("user cache\t removed {}({} cached)",remove_targets.len(),user_cache.len());
		remove_targets.clear();
		for (k,v) in instance_cache.iter(){
			let count=Arc::strong_count(v);
			if count<rc{
				remove_targets.push(k.clone());
			}
		}
		for r in &remove_targets{
			instance_cache.remove(r);
		}
		println!("instance cache\t removed {}({} cached)",remove_targets.len(),instance_cache.len());
		remove_targets.clear();
	}
	//脱出するとspawnしたジョブが破棄されるので適当に待つ
	//tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;
}
async fn load_note(
	mut note:RawNote,
	note_cache:&mut HashMap<String, Arc<data_model::Note>>,
	user_cache:&mut HashMap<String, Arc<data_model::UserProfile>>,
	instance_cache:&mut HashMap<String, Arc<data_model::FediverseInstance>>,
	file_cache:&mut HashMap<String, NoteFile>,
	local_instance:impl AsRef<str>,
	emoji_cache:&EmojiCache,
)->Option<data_model::Note>{
	println!("load note {}",note.id);
	// Print the text of the note, if any.
	fn format_timestamp(unix_sec:i64)->String{
		let secs_ago=Utc::now().timestamp()-unix_sec;
		if secs_ago>12*30*24*60*60{
			format!("{}年前",secs_ago/(12*30*24*60*60))
		}else if secs_ago>30*24*60*60{
			format!("{}ヶ月前",secs_ago/(30*24*60*60))
		}else if secs_ago>7*24*60*60{
			format!("{}週間前",secs_ago/(7*24*60*60))
		}else if secs_ago>24*60*60{
			format!("{}日前",secs_ago/(24*60*60))
		}else if secs_ago>60*60{
			format!("{}時間前",secs_ago/(60*60))
		}else if secs_ago>60{
			format!("{}分前",secs_ago/60)
		}else{
			format!("{}秒前",secs_ago)
		}
	}
	async fn note_user(
		user_cache:&mut HashMap<String,Arc<data_model::UserProfile>>,
		instance_cache: &mut HashMap<String, Arc<data_model::FediverseInstance>>,
		user:&RawUser,
		local_instance:impl AsRef<str>,
		emoji_cache:&data_model::EmojiCache,
	)->Arc<data_model::UserProfile>{
		let user_id=&user.id;
		match user_cache.get(user_id){
			Some(hit)=>hit.clone(),
			None=>{
				let user=data_model::UserProfile::load(&user,instance_cache,local_instance,emoji_cache).await;
				let user=Arc::new(user);
				user_cache.insert(user_id.to_owned(),user.clone());
				user
			}
		}
	}
	fn note_files(note:&RawNote,file_cache:&mut HashMap<String,data_model::NoteFile>)->Vec<NoteFile>{
		let mut files=vec![];
		for f in note.files.iter(){
			if let Some(hit)=file_cache.get(&f.id){
				files.push(hit.clone());
			}else{
				let id=f.id.to_owned();
				let f=NoteFile::from(f);
				file_cache.insert(id,f.clone());
				files.push(f);
			}
		}
		files
	}
	if let Some(quote)=note.renote.take(){
		//引用されたノート
		let n=if let Some(n)=note_cache.get(&quote.id){
			n.clone()
		}else{
			let user=note_user(user_cache,instance_cache,&quote.user,&local_instance,&emoji_cache).await;
			let reactions=data_model::Reactions::load(&quote,&emoji_cache).await;
			let created_at=quote.created_at();
			let n=Arc::new(crate::data_model::Note{
				quote:None,
				created_at,
				time_label:format_timestamp(created_at.timestamp()),
				visibility:quote.visibility.as_str().into(),
				reactions,
				files:note_files(&quote,file_cache),
				text:data_model::MFMString::new(quote.text.unwrap_or_default(),quote.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
				id:quote.id,
				cw:data_model::MFMString::new_opt(quote.cw,quote.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
				user,
			});
			note_cache.insert(n.id.to_owned(),n.clone());
			n
		};
		let user=note_user(user_cache,instance_cache,&note.user,&local_instance,&emoji_cache).await;
		let reactions=data_model::Reactions::load(&note,&emoji_cache).await;
		let created_at=note.created_at();
		Some(crate::data_model::Note{
			quote:Some(n),
			created_at,
			time_label:format_timestamp(created_at.timestamp()),
			visibility:note.visibility.as_str().into(),
			reactions,
			files:note_files(&note,file_cache),
			text:data_model::MFMString::new(note.text.unwrap_or_default(),note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			id:note.id,
			cw:data_model::MFMString::new_opt(note.cw,note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			user,
		})
	}else if note.text.is_some() {
		let user=note_user(user_cache,instance_cache,&note.user,local_instance,&emoji_cache).await;
		let reactions=data_model::Reactions::load(&note,&emoji_cache).await;
		let created_at=note.created_at();
		Some(crate::data_model::Note{
			quote:None,
			created_at,
			time_label:format_timestamp(created_at.timestamp()),
			visibility:note.visibility.as_str().into(),
			reactions,
			files:note_files(&note,file_cache),
			text:data_model::MFMString::new(note.text.unwrap(),note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			id:note.id,
			cw:data_model::MFMString::new_opt(note.cw,note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			user,
		})
	}else{
		None
	}
}
#[derive(Serialize,Deserialize,Debug)]
struct RawEmojis{
	emojis:Vec<RawEmojiElement>
}
#[derive(Serialize,Deserialize,Debug)]
struct RawEmojiElement{
	name:String,
	category:Option<String>,
	url:String,
}
async fn emojis(client:&Client,local_instance:&str)->Result<RawEmojis,String>{
	let req_builder=client.get(format!("{}/api/emojis",local_instance));
	let meta=req_builder.send().await.map_err(|e|e.to_string())?;
	let meta=meta.bytes().await.map_err(|e|e.to_string())?;
	serde_json::from_slice(&meta).map_err(|e|e.to_string())
}
#[derive(Serialize,Deserialize,Debug)]
struct ApiMeta{
	ads:Vec<serde_json::Value>,
	#[serde(rename = "backgroundImageUrl")]
	background_image_url:Option<String>,
	#[serde(rename = "bannerUrl")]
	banner_url:Option<String>,
	#[serde(rename = "cacheRemoteFiles")]
	cache_remote_files:Option<bool>,
	#[serde(rename = "cacheRemoteSensitiveFiles")]
	cache_remote_sensitive_files:Option<bool>,
	description:Option<String>,
	#[serde(rename = "disableRegistration")]
	disable_registration:Option<bool>,
	#[serde(rename = "emailRequiredForSignup")]
	email_required_for_signup:Option<bool>,
	#[serde(rename = "enableEmail")]
	enable_email:Option<bool>,
	#[serde(rename = "enableHcaptcha")]
	enable_hcaptcha:Option<bool>,
	#[serde(rename = "enableMcaptcha")]
	enable_mcaptcha:Option<bool>,
	#[serde(rename = "enableRecaptcha")]
	enable_recaptcha:Option<bool>,
	#[serde(rename = "enableTurnstile")]
	enable_turnstile:Option<bool>,
	features:MetaFeatures,
	#[serde(rename = "feedbackUrl")]
	feedback_url:Option<String>,
	#[serde(rename = "impressumUrl")]
	impressum_url:Option<String>,
	#[serde(rename = "infoImageUrl")]
	info_image_url:Option<String>,
	#[serde(rename = "maintainerEmail")]
	maintainer_email:Option<String>,
	#[serde(rename = "maintainerName")]
	maintainer_name:Option<String>,
	#[serde(rename = "maxNoteTextLength")]
	max_note_text_length:Option<u64>,
	#[serde(rename = "mcaptchaInstanceUrl")]
	mcaptcha_instance_url:Option<String>,
	#[serde(rename = "mediaProxy")]
	media_proxy:String,
	#[serde(rename = "name")]
	instance_name:Option<String>,
	#[serde(rename = "notFoundImageUrl")]
	not_found_image_url:Option<String>,
}
#[derive(Serialize,Deserialize,Debug)]
struct MetaFeatures{
	#[serde(rename = "globalTimeline")]
	global_timeline:Option<bool>,
	#[serde(rename = "localTimeline")]
	local_timeline:Option<bool>,
	miauth:Option<bool>,
	#[serde(rename = "objectStorage")]
	object_storage:Option<bool>,
}
async fn meta(client:&Client,local_instance:&str)->Result<ApiMeta,String>{
	let req_builder=client.post(format!("{}/api/meta",local_instance));
	let req_builder=req_builder.header(reqwest::header::CONTENT_TYPE,"application/json");
	let req_builder=req_builder.header(reqwest::header::CONTENT_LENGTH,2);
	let req_builder=req_builder.body("{}");
	let meta=req_builder.send().await.map_err(|e|e.to_string())?;
	let meta=meta.bytes().await.map_err(|e|e.to_string())?;
	serde_json::from_slice(&meta).map_err(|e|e.to_string())
}
#[derive(Serialize,Deserialize,Debug)]
struct TimelineRequestJson{
	#[serde(rename = "allowPartial")]
	allow_partial:bool,
	#[serde(rename = "withRenotes")]
	with_renotes:bool,
	limit:u8,
	i:String,
}
#[derive(Clone,Debug)]
pub enum TimeLine{
	Global,
	Home,
}
impl ToString for TimeLine{
	fn to_string(&self) -> String {
		match self {
			TimeLine::Global => "global-timeline",
			TimeLine::Home => "timeline",
		}.to_owned()
	}
}
async fn read_timeline(client:&Client,local_instance:&str,token:String,(limit,tl):(u8,TimeLine))->Result<Vec<RawNote>,String>{
	let req_builder=client.post(format!("{}/api/notes/{}",local_instance,tl.to_string()));
	//global-timeline
	//timeline
	let req_builder=req_builder.header(reqwest::header::CONTENT_TYPE,"application/json");
	let req_body=TimelineRequestJson{
		allow_partial: true,
		with_renotes: true,
		limit,
		i:token,
	};
	let req_body=serde_json::to_string(&req_body).map_err(|e|e.to_string())?;
	let req_builder=req_builder.header(reqwest::header::CONTENT_LENGTH,req_body.len());
	let req_builder=req_builder.body(req_body);
	let htl=req_builder.send().await;
	let htl=htl.map_err(|e|e.to_string())?;
	if htl.status()!=200{
		return Err(format!("post status {}",htl.status()))
	}
	let htl=htl.bytes().await;
	let htl=htl.map_err(|e|e.to_string())?;
	let htl=serde_json::from_slice(&htl);
	let htl=htl.map_err(|e|e.to_string())?;
	Ok(htl)
}
#[derive(Serialize,Deserialize,Debug)]
pub struct RawNote{
	id:String,
	text:Option<String>,
	#[serde(rename = "createdAt")]
	created_at:String,
	cw:Option<String>,
	emojis:Option<HashMap<String,String>>,
	#[serde(rename = "fileIds")]
	file_ids:Vec<String>,
	files:Vec<RawFile>,
	#[serde(rename = "localOnly")]
	local_only:Option<bool>,
	#[serde(rename = "reactionEmojis")]
	pub reaction_emojis:HashMap<String,String>,
	pub reactions:HashMap<String,u64>,
	#[serde(rename = "renoteCount")]
	renote_count:u64,
	renote:Option<Box<RawNote>>,
	#[serde(rename = "repliesCount")]
	replies_count:u64,
	#[serde(rename = "uri")]
	remote_uri:Option<String>,
	user:RawUser,
	visibility:String,
	user_id:Option<String>,
}
impl RawNote{
	fn created_at(&self)->chrono::DateTime<chrono::Utc>{
		chrono::DateTime::parse_from_rfc3339(&self.created_at).unwrap().to_utc()
	}
}
#[derive(Serialize,Deserialize,Debug)]
pub struct RawFile{
	pub id:String,//9t8xrxeg8f
	pub blurhash:Option<String>,//eHRomc?]RP%2pHoZVZtRtRVtWTV@RPayV@-;tRWAtRoz.SjboykBV@
	pub comment:Option<String>,
	#[serde(rename = "createdAt")]
	pub created_at:String,//2024-05-13T19:43:44.344Z
	pub folder:Option<String>,//null
	#[serde(rename = "folderId")]
	pub folder_id:Option<String>,//null
	#[serde(rename = "isSensitive")]
	pub is_sensitive:bool,//false
	pub md5:Option<String>,//2eac361d306ec9f62046df9670ed12e8
	pub name:Option<String>,//2059fec9bb7f6e1de29c2154500ddc5a6deae427c260e17e3544c994542fc0fd.jpg 
	pub properties:Option<RawFileProperties>,
	pub size: u64,
	#[serde(rename = "thumbnailUrl")]
	pub thumbnail_url:Option<String>,//https://misskey.kzkr.xyz/proxy/static.webp?url=https%3A%2F%2Fghetti.monster%2Fmedia%2F2059fec9bb7f6e1de29c2154500ddc5a6deae427c260e17e3544c994542fc0fd.jpg&static=1
	pub url:Option<String>,//https://misskey.kzkr.xyz/files/webpublic-2381cdd3-10b3-4bd1-8dc4-5301a95ebed1
	#[serde(rename = "type")]
	pub mime_type: Option<String>,//image/jpeg 
	pub user: Option<RawUser>,//null
	#[serde(rename = "userId")]
	pub user_id:Option<String>,//null
}
#[derive(Serialize,Deserialize,Debug)]
pub struct RawFileProperties{
	pub width:Option<u32>,//1414
	pub height:Option<u32>,//1000
}
#[derive(Serialize,Deserialize,Debug)]
pub struct RawUser{
	#[serde(rename = "avatarBlurhash")]
	pub avatar_blurhash:Option<String>,
	pub avatar_decorations:Option<Vec<serde_json::Value>>,
	#[serde(rename = "avatarUrl")]
	pub avatar_url:Option<String>,
	pub emojis:Option<HashMap<String,String>>,
	pub host:Option<String>,
	pub id:String,
	pub name:Option<String>,
	pub username:String,
	#[serde(rename = "onlineStatus")]
	pub online_status:Option<String>,
	pub instance:Option<RawInstance>,
	pub is_bot:Option<bool>,
	pub is_cat:Option<bool>,
	pub is_fox:Option<bool>,
}
#[derive(Serialize,Deserialize,Debug)]
pub struct RawInstance{
	#[serde(rename = "faviconUrl")]
	pub favicon_url:Option<String>,
	#[serde(rename = "iconUrl")]
	pub icon_url:Option<String>,
	pub name:Option<String>,
	#[serde(rename = "softwareName")]
	pub software_name:Option<String>,
	#[serde(rename = "softwareVersion")]
	pub software_version:Option<String>,
	#[serde(rename = "themeColor")]
	pub theme_color:Option<String>,
}

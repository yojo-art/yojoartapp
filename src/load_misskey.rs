
use std::{collections::HashMap, hash::{Hash, Hasher}, sync::{atomic::{AtomicBool, AtomicU32}, Arc}};

use futures::{ SinkExt, StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc::{Receiver, Sender}, Mutex};

use crate::{data_model::{self, DelayAssets, EmojiCache, NoteFile}, ConfigFile};

pub struct TLOption{
	pub(crate) limit:u8,
	pub(crate) tl:TimeLine,
	pub(crate) known_notes:Vec<Arc<data_model::Note>>,
	pub(crate) websocket:bool,
}
pub enum LoadSrc{
	TimeLine(TLOption),
	Note(String),
}
pub async fn load_misskey(
	config:Arc<ConfigFile>,
	note_ui:Sender<Arc<data_model::Note>>,
	delay_assets:Sender<DelayAssets>,
	client:Client,
	mut reload_event:Receiver<LoadSrc>,
	emojis_send:Sender<EmojiCache>,
){
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
	let local_instance=config.instance.clone().unwrap();
	let meta=meta(&client,&local_instance).await;
	if let Err(e)=meta{
		let mes=format!("get api/meta error {}",e);
		if let Err(e)=note_ui.send(Arc::new(data_model::Note::system_message(mes,"").await)).await{
			eprintln!("{:?}",e);
		}
		return;
	}
	let meta=meta.unwrap();
	let media_proxy=meta.media_proxy;
	println!("media_proxy:{}",media_proxy);
	let mut local_emojis=HashMap::new();
	if let Ok(emojis)=emojis(&client,&local_instance).await{
		for emoji in emojis.emojis{
			local_emojis.insert(emoji.name,emoji.url);
		}
	}
	println!("{} local emojis",local_emojis.len());
	let emoji_cache=data_model::EmojiCache::new(media_proxy,&local_instance,Arc::new(local_emojis));
	let _=emojis_send.send(emoji_cache.clone()).await;
	let mut instance_cache=HashMap::new();
	let mut user_cache=HashMap::new();
	let mut file_cache=HashMap::new();
	let mut note_cache: HashMap<String, Arc<data_model::Note>>=HashMap::new();
	let (raw_note_sender,mut raw_note_receiver)=tokio::sync::mpsc::channel(4);
	let note_ui0=note_ui.clone();
	tokio::runtime::Handle::current().spawn(async move{
		let mut state=WSState{
			stream: None,
			now_stream: None,
		};
		while let Some(limit) = reload_event.recv().await{
			match limit{
				LoadSrc::TimeLine(limit) => {
					eprintln!("read_websocket {:?}",read_websocket(config.clone(),raw_note_sender.clone(),if limit.websocket{
						Some(limit.tl.into())
					}else{
						None
					},&mut state).await);
					let limit=(limit.limit,limit.tl,limit.known_notes);
					let htl=read_timeline(&client,config.instance.as_ref().unwrap(),config.token.as_ref().unwrap().clone(),limit.clone()).await;
					if let Err(e)=htl{
						let mes=format!("get api/notes/{} error {}",limit.1.to_string(),e);
						if let Err(e)=note_ui0.send(Arc::new(data_model::Note::system_message(mes,"").await)).await{
							eprintln!("{:?}",e);
						}
					}else{
						let notes=htl.unwrap();
						println!("{} notes get",notes.len());
						if let Err(e)=raw_note_sender.send(RawNotes::Array(notes)).await{
							eprintln!("{:?}",e);
						}
					}
				},
				LoadSrc::Note(note_id) => {
					let note=client.post(format!("{}/api/notes/show",config.instance.as_ref().unwrap()));
					#[derive(Serialize,Debug)]
					struct ShowPayload{
						i:String,
						#[serde(rename = "noteId")]
						note_id:String,
					}
					let note=note.header(reqwest::header::CONTENT_TYPE,"application/json");
					let note=note.body(serde_json::to_string(&ShowPayload{
						i:config.token.as_ref().unwrap().clone(),
						note_id,
					}).unwrap());
					match note.send().await{
						Ok(json) => {
							println!("{}",json.status());
							if let Ok(json)=json.bytes().await{
								if let Ok(note)=serde_json::from_slice::<RawNote>(&json){
									println!("RAW NOTE");
									if let Err(e)=raw_note_sender.send(RawNotes::Single(note)).await{
										let _=note_ui0.send(Arc::new(data_model::Note::system_message(format!("{:?}",e),"").await)).await;
									}
								}
							}
						},
						Err(e) => {
							let _=note_ui0.send(Arc::new(data_model::Note::system_message(format!("{:?}",e),"").await)).await;
						},
					}
				},
			}
		}
	});
	let mut cacche_clean_wait_count=15;
	while let Some(notes) = raw_note_receiver.recv().await{
		match notes{
			RawNotes::Single(note) =>{
				if let Some((n,is_cache)) = load_note(note,&mut note_cache,&mut user_cache,&mut instance_cache,&mut file_cache,&emoji_cache).await {
					//println!("@{}: {}", note.user.username, text);
					note_cache.insert(n.id.to_owned(),n.clone());
					let note=n.clone();
					let note_ui=note_ui.clone();
					if let Err(e)=note_ui.send(note).await{
						eprintln!("{:?}",e);
					}
					if !is_cache{
						let delay_assets=delay_assets.clone();
						tokio::runtime::Handle::current().spawn(async move{
							if let Err(e)=delay_assets.send(DelayAssets::Note(n)).await{
								eprintln!("{:?}",e);
							}
						});
						cacche_clean_wait_count-=1;
					}
				}
			},
			RawNotes::Array(notes) => {
				let mut note_load=vec![];
				for note in notes{
					if let Some((n,is_cache)) = load_note(note,&mut note_cache,&mut user_cache,&mut instance_cache,&mut file_cache,&emoji_cache).await {
						//println!("@{}: {}", note.user.username, text);
						note_cache.insert(n.id.to_owned(),n.clone());
						let note=n.clone();
						let note_ui=note_ui.clone();
						if let Err(e)=note_ui.send(note).await{
							eprintln!("{:?}",e);
						}
						if !is_cache{
							note_load.push(n);
							cacche_clean_wait_count-=1;
						}
					}
				}
				for n in note_load.into_iter().rev(){
					let delay_assets=delay_assets.clone();
					tokio::runtime::Handle::current().spawn(async move{
						if let Err(e)=delay_assets.send(DelayAssets::Note(n)).await{
							eprintln!("{:?}",e);
						}
					});
				}
			},
		}
		if cacche_clean_wait_count>0{
			continue;
		}
		cacche_clean_wait_count=15;
		let rc=2;//削除しきい値
		let mut remove_targets=vec![];
		let mut removed_note=0;
		for _ in 0..3{
			for (k,v) in note_cache.iter(){
				let count=Arc::strong_count(v);
				if count<rc{
					remove_targets.push(k.clone());
				}
			}
			if remove_targets.is_empty(){
				break;
			}
			for r in &remove_targets{
				note_cache.remove(r);
			}
			removed_note+=remove_targets.len();
			remove_targets.clear();
		};
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
struct WSState{
	stream:Option<Arc<WSStream>>,
	now_stream:Option<u32>,
}
async fn read_websocket(config:Arc<ConfigFile>,sender:tokio::sync::mpsc::Sender<RawNotes>,v:Option<MisskeyChannel>,state:&mut WSState)->Result<(),reqwest_websocket::Error>{
	if let Some(ch)=v{
		if state.stream.is_none(){
			state.stream=Some({
				use reqwest_websocket::RequestBuilderExt;
				let url=reqwest::Url::parse(config.instance.as_ref().unwrap());
				let mut url=match url {
					Ok(url)=>url,
					Err(e)=>{
						eprintln!("{:?}",e);
						return Ok(());
					}
				};
				if url.scheme()=="http"{
					url.set_scheme("ws").unwrap();
				}else{
					url.set_scheme("wss").unwrap();
				}
				url.set_path("streaming");
				let query=format!("i={}",config.token.as_ref().unwrap());
				url.set_query(Some(&query));
				// create a GET request, upgrade it and send it.
				let response = Client::default()
					.get(url)
					.upgrade() // <-- prepares the websocket upgrade.
					.send()
					.await?;
				let websocket = response.into_websocket().await?;
				let ws=Arc::new(WSStream::new(websocket));
				let ws0=ws.clone();
				tokio::runtime::Handle::current().spawn(async move{
					let _=ws0.load().await;
				});
				println!("=============Open Connection===============");
				ws
			});
		}
		let sender=sender.clone();
		println!("=============Open Stream===============");
		state.stream.as_ref().unwrap().open(move|res: WSChannel|{
			let sender=sender.clone();
			let f:futures::future::BoxFuture<'static,()>=Box::pin(async move{
				if res.t.as_str()=="note"{
					if let Ok(note)=serde_json::value::from_value::<RawNote>(res.body){
						if let Err(e)=sender.send(RawNotes::Single(note)).await{
							eprintln!("{:?}",e);
						}
					}
				}
			});
			f
		},ch).await?;
		if let Some(id)=state.now_stream{
			if state.stream.as_ref().unwrap().close_channel(id).await.is_ok(){
				state.now_stream=Some(id);
			}
		}
	}else{
		if let Some(id)=state.now_stream.take(){
			if let Err(e)=state.stream.as_ref().unwrap().close_channel(id).await{
				println!("close stream error {:?}",e);
			}
		}
		if let Some(stream)=state.stream.take(){
			stream.close_connection().await;
		}
	}
	Ok(())
}
enum RawNotes{
	Single(RawNote),
	Array(Vec<RawNote>),
}
struct WSChannelListener(Box<dyn FnMut(WSChannel)->futures::future::BoxFuture<'static, ()>+Send+Sync>);
impl <F> From<F> for WSChannelListener where F:FnMut(WSChannel)->futures::future::BoxFuture<'static, ()>+Send+Sync+'static{
	fn from(value: F) -> Self {
		Self(Box::new(value))
	}
}
pub enum MisskeyChannel{
	GlobalTimeline,
	HomeTimeline,
}
impl MisskeyChannel{
	pub fn id(&self)->&'static str{
		match self {
			MisskeyChannel::GlobalTimeline => "globalTimeline",
			MisskeyChannel::HomeTimeline => "homeTimeline",
		}
	}
}
impl From<TimeLine> for MisskeyChannel{
	fn from(value: TimeLine) -> Self {
		match value {
			TimeLine::Global => Self::GlobalTimeline,
			TimeLine::Home => Self::HomeTimeline,
		}
	}
}
struct WSStream{
	channel_listener:Arc<Mutex<HashMap<u32,WSChannelListener>>>,
	last_id:AtomicU32,
	send: Arc<Mutex<futures::prelude::stream::SplitSink<reqwest_websocket::WebSocket, reqwest_websocket::Message>>>,
	recv: Mutex<Option<futures::prelude::stream::SplitStream<reqwest_websocket::WebSocket>>>,
	exit: Arc<AtomicBool>,
}
impl WSStream{
	fn new(websocket:reqwest_websocket::WebSocket)->Self{
		let (send,recv)=websocket.split();
		Self{
			channel_listener:Arc::new(Mutex::new(HashMap::new())),
			last_id:AtomicU32::new(0),
			send:Arc::new(Mutex::new(send)),
			recv:Mutex::new(Some(recv)),
			exit:Arc::new(AtomicBool::new(false)),
		}
	}
	async fn open(&self,listener:impl Into<WSChannelListener>,channel:MisskeyChannel)->Result<u32,reqwest_websocket::Error>{
		let mut websocket=self.send.lock().await;
		let id=self.last_id.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
		println!("open channel... {}",id);
		let mut channel_listener=self.channel_listener.lock().await;
		channel_listener.insert(id,listener.into());
		let q=format!("{{\"type\":\"connect\",\"body\":{{\"channel\":\"{}\",\"id\":\"{}\",\"params\":{{\"withRenotes\":true,\"withCats\":false}}}}}}",channel.id(),id);
		websocket.send(reqwest_websocket::Message::Text(q.into())).await?;
		println!("opend channel {}",id);
		Ok(id)
	}
	async fn close_channel(&self,id:u32)->Result<u32,reqwest_websocket::Error>{
		println!("close channel... {}",id);
		let mut websocket=self.send.lock().await;
		let q=format!("{{\"type\":\"disconnect\",\"body\":{{\"id\":\"{}\"}}}}",id);
		websocket.send(reqwest_websocket::Message::Text(q.into())).await?;
		let mut channel_listener=self.channel_listener.lock().await;
		channel_listener.remove(&id);
		println!("closed channel {}",id);
		Ok(id)
	}
	async fn load(&self){
		let websocket=self.recv.lock().await.take();
		if websocket.is_none(){
			return;
		}
		let mut websocket=websocket.unwrap();
		let channel_listener=self.channel_listener.clone();
		let sender=self.send.clone();
		let exit0=self.exit.clone();
		std::thread::spawn(move||{
			let rt=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
			let handle=rt.spawn(async move{
				while let Ok(Some(message)) = websocket.try_next().await {
					match message {
						reqwest_websocket::Message::Text(text) =>{
							if let Ok(Some(channel))=serde_json::from_str::<WSResult>(text.as_str()).map(|res|{
								if res.t.as_str()=="channel"{
									serde_json::value::from_value::<WSChannel>(res.body).ok()
								}else{
									None
								}
							}){
								if let Ok(id)=u32::from_str_radix(channel.id.as_str(),10){
									let mut r=channel_listener.lock().await;
									if let Some(handle)=r.get_mut(&id){
										handle.0(channel).await;
									}else{
										println!("unknown channel event {}",id);
									}
								}
							}else{
								println!("parse error {}",text);
							}
						},
						_=>{}
					}
				}
				println!("close websocket");
			});
			rt.block_on(async{
				while !exit0.load(std::sync::atomic::Ordering::Relaxed){
					let mut websocket=sender.lock().await;
					if let Err(e)=websocket.send(reqwest_websocket::Message::Text("h".into())).await{
						println!("ping error {:?}",e);
					}else{
						println!("ping ok");
					}
					drop(websocket);
					tokio::time::sleep(tokio::time::Duration::from_millis(60*1000)).await;
				}
			});
			handle.abort();
		});
	}
	async fn close_connection(&self){
		println!("close connection...");
		self.exit.store(true,std::sync::atomic::Ordering::Relaxed);
		let mut websocket=self.send.lock().await;
		let res=websocket.close().await;
		println!("closed connection {:?}",res);
	}
}
#[derive(Serialize,Deserialize,Debug)]
struct WSResult{
	#[serde(rename = "type")]
	t:String,
	body:serde_json::Value,
}
#[derive(Serialize,Deserialize,Debug)]
struct WSChannel{
	#[serde(rename = "type")]
	t:String,
	id:String,
	body:serde_json::Value,
}
async fn load_note(
	mut note:RawNote,
	note_cache:&mut HashMap<String, Arc<data_model::Note>>,
	user_cache:&mut HashMap<String, Arc<data_model::UserProfile>>,
	instance_cache:&mut HashMap<String, Arc<data_model::FediverseInstance>>,
	file_cache:&mut HashMap<String, NoteFile>,
	emoji_cache:&EmojiCache,
)->Option<(Arc<data_model::Note>,bool)>{
	if let Some(n)=note_cache.get(&note.id){
		let hash=reactions_hash(&note);
		if n.reactions.hash==hash{
			return Some((n.clone(),true));
		}
	}
	println!("load note {}",note.id);
	// Print the text of the note, if any.
	async fn note_user(
		user_cache:&mut HashMap<String,Arc<data_model::UserProfile>>,
		instance_cache: &mut HashMap<String, Arc<data_model::FediverseInstance>>,
		user:&RawUser,
		emoji_cache:&data_model::EmojiCache,
	)->Arc<data_model::UserProfile>{
		let user_id=&user.id;
		match user_cache.get(user_id){
			Some(hit)=>hit.clone(),
			None=>{
				let user=data_model::UserProfile::load(&user,instance_cache,emoji_cache).await;
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
			let user=note_user(user_cache,instance_cache,&quote.user,&emoji_cache).await;
			let reactions=data_model::Reactions::load(&quote,&emoji_cache).await;
			let created_at=quote.created_at();
			let n=Arc::new(crate::data_model::Note{
				quote:None,
				created_at,
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
		let user=note_user(user_cache,instance_cache,&note.user,&emoji_cache).await;
		let reactions=data_model::Reactions::load(&note,&emoji_cache).await;
		let created_at=note.created_at();
		Some((Arc::new(crate::data_model::Note{
			quote:Some(n),
			created_at,
			visibility:note.visibility.as_str().into(),
			reactions,
			files:note_files(&note,file_cache),
			text:data_model::MFMString::new(note.text.unwrap_or_default(),note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			id:note.id,
			cw:data_model::MFMString::new_opt(note.cw,note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			user,
		}),false))
	}else if note.text.is_some() {
		let user=note_user(user_cache,instance_cache,&note.user,&emoji_cache).await;
		let reactions=data_model::Reactions::load(&note,&emoji_cache).await;
		let created_at=note.created_at();
		Some((Arc::new(crate::data_model::Note{
			quote:None,
			created_at,
			visibility:note.visibility.as_str().into(),
			reactions,
			files:note_files(&note,file_cache),
			text:data_model::MFMString::new(note.text.unwrap(),note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			id:note.id,
			cw:data_model::MFMString::new_opt(note.cw,note.emojis.as_ref(),user.instance.as_ref(),&emoji_cache).await,
			user,
		}),false))
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
#[derive(PartialEq,Eq,Copy,Clone,Debug)]
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
async fn read_timeline(client:&Client,local_instance:&str,token:String,(limit,tl,knwon):(u8,TimeLine,Vec<Arc<data_model::Note>>))->Result<Vec<RawNote>,String>{
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
	let req_builder=req_builder.timeout(std::time::Duration::from_secs(5));
	let htl=req_builder.send().await;
	let htl=htl.map_err(|e|e.to_string())?;
	if htl.status()!=200{
		return Err(format!("post status {}",htl.status()))
	}
	let htl=htl.bytes().await;
	let htl=htl.map_err(|e|e.to_string())?;
	let htl=serde_json::from_slice(&htl);
	let htl: Vec<RawNote>=htl.map_err(|e|e.to_string())?;
	let mut known_notes_map=HashMap::new();
	for note in knwon{
		known_notes_map.insert(note.id.as_str().to_owned(),note);
	}
	let mut htl_update=vec![];
	for note in htl.into_iter().rev(){
		if let Some(hit)=known_notes_map.get(note.id.as_str()){
			let hash=reactions_hash(&note);
			if hit.reactions.hash==hash{
				continue;
			}
		}
		htl_update.push(note);
	}
	println!("INSERT {}",htl_update.len());
	Ok(htl_update)
}
fn reactions_hash(note:&RawNote)->u64{
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	let mut hash=0;
	for (id,r) in &note.reactions{
		id.hash(&mut hasher);
		hash+=*r;
	}
	hash+=hasher.finish();
	hash
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

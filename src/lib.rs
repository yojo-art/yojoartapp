#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data_model;
mod load_misskey;
mod gui;
mod delay_assets;
use std::{io::Write, sync::Arc};

use data_model::Visibility;
use eframe::NativeOptions;

use serde::{Deserialize, Serialize};
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
#[derive(Eq,PartialEq,Clone,Debug,Default,Serialize,Deserialize)]
pub enum FileThumbnailMode{
	#[default]
	Thumbnail,
	None,
	Original
}
#[derive(Clone,Debug,Default,Serialize,Deserialize)]
pub struct StateFile{
	timeline: load_misskey::TimeLine,
	until_id:Option<String>,
	nsfw_always_show:bool,
	auto_old_timeline:bool,
	file_thumbnail_mode:FileThumbnailMode,
	default_renote_visibility:Visibility,
}
impl StateFile{
	fn file()->String{
		match std::env::var("YAC_STATE_PATH"){
			Ok(path)=>{
				if path.is_empty(){
					"state.json".to_owned()
				}else{
					path
				}
			},
			Err(_)=>"state.json".to_owned()
		}
	}
	pub fn write(&self,delay_assets:&tokio::sync::mpsc::Sender<data_model::DelayAssets>){
		if let Ok(writer)=std::fs::File::create(Self::file()){
			if let Err(e)=serde_json::to_writer(writer,&self){
				eprintln!("{:?}",e);
			}
		}
		let v=Arc::new(self.clone());
		let _=delay_assets.blocking_send(data_model::DelayAssets::UpdateState(v));
	}
	pub fn load()->Option<Self>{
		if let Ok(writer)=std::fs::File::open(Self::file()){
			match serde_json::from_reader(writer){
				Ok(d)=>return Some(d),
				Err(e)=>eprintln!("{:?}",e)
			}
		}
		None
	}
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
	add_reaction:String,
	reload:String,
	open_in_browser:String,
	summaly_default_title:String,
	summaly_default_description:String,
	summaly_default_sitename:String,
	thumbnail_mode:String,
	default_thumbnail_img:String,
	always_original_img:String,
	no_thumbnail_img:String,
	visibility_public:String,
	visibility_home:String,
	visibility_followers:String,
	visibility_specified:String,
	send_renote:String,
	default_renote_visibility:String,
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
fn common<F>(options:NativeOptions,ime_show:F)where F:FnMut(&mut bool)+'static{
	/*
	let emoji_src=include_str!("include/unicodeemoji.txt").split("\n");
	let mut f=std::fs::File::create("unicodeemoji.utf32").unwrap();
	f.write_all(&[00,00,0xFE,0xFF]).unwrap();
	for emoji in emoji_src{
		let c=u32::from_str_radix(&emoji[2..],16).unwrap();
		f.write_all(&c.to_be_bytes()).unwrap();
		//let c=char::from_u32(c).unwrap();
	}
	drop(f);
	*/
	gui::main_ui::open(options,ime_show);
}

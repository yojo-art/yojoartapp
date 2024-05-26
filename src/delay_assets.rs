use std::sync::Arc;

use crate::{data_model, ConfigFile};

use reqwest::Client;
use tokio::sync::mpsc::Receiver;

pub(crate) async fn delay_assets(mut recv:Receiver<data_model::DelayAssets>,ctx:egui::Context,client:Client,config:Arc<ConfigFile>){
	//tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
	let mut note_buf=Vec::with_capacity(4);
	let mut job_buf=Vec::with_capacity(4);
	let mut emoji_job_buf=Vec::with_capacity(4);
	loop{
		let limit=note_buf.capacity();
		if recv.recv_many(&mut note_buf,limit).await==0{
			return;
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
async fn load_note(note:Arc<data_model::Note>,ctx:&egui::Context,client:&Client,config:&Arc<ConfigFile>){
	let mut job_buf=Vec::with_capacity(4);
	let mut job_buf2=Vec::with_capacity(4);
	let mut job_buf_emojis=Vec::with_capacity(32);
	let mut job_buf_urls=Vec::new();
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
	for (url,summaly) in note.text.urls(){
		let summaly_server="https://summaly.xn--vusz0j.art/";
		let summaly=summaly.clone();
		let url=url.clone();
		job_buf_urls.push(async move{
			if let Some(res)=data_model::Summaly::load(&client,&summaly_server,&url).await{
				let mut lock=summaly.lock().await;
				let thumbnail=res.thumbnail.as_ref().cloned();
				let icon=res.icon.as_ref().cloned();
				lock.replace(res);
				drop(lock);
				futures::join!(
					async{
						if let Some(thumbnail)=thumbnail{
							thumbnail.load(&client).await;
							thumbnail.load_gpu(&ctx,&config).await;
						}
					},
					async{
						if let Some(icon)=icon{
							icon.load(&client).await;
							icon.load_gpu(&ctx,&config).await;
						}
					},
				);
			}
		});
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
		futures::future::join_all(job_buf_urls.drain(..)),
	);
}
async fn load_url(emoji:Arc<data_model::UrlImage>,client: reqwest::Client,ctx: egui::Context,config:Arc<ConfigFile>){
	//tokio::time::sleep(tokio::time::Duration::from_millis(10000)).await;
	emoji.load(&client).await;
	emoji.load_gpu(&ctx,&config).await;
}

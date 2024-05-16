use egui_winit::winit::platform::android::activity::AndroidApp;

#[no_mangle]
#[cfg(target_os="android")]
pub fn android_main(app: AndroidApp){
	env_logger::init();
	eprintln!("android_main");
	{
		let jvm=unsafe{
			jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _).unwrap()
		};
		let activity = unsafe{
			jni::objects::JObject::from_raw(app.activity_as_ptr() as *mut _)
		};
		let mut env=jvm.attach_current_thread().unwrap();
		let file=env.call_method(activity,"getCacheDir","()Ljava/io/File;",&[]).unwrap();
		let file=file.l().unwrap();
		let path=env.call_method(file,"getAbsolutePath","()Ljava/lang/String;",&[]).unwrap();
		let s=path.l().unwrap();
		let s=jni::objects::JString::from(s);
		let s=env.get_string(&s).unwrap();
		let s=s.to_string_lossy().to_string();
		std::env::set_var("YAC_CACHE_PATH", s);
	}
	let app2=app.clone();
	if let Some(internal_data_path)=app.internal_data_path(){
		let value=internal_data_path.clone().join("config.json");
		std::env::set_var("YAC_CONFIG_PATH", value);
	}
	let options = eframe::NativeOptions {
		event_loop_builder: Some(Box::new(move |builder| {
			use egui_winit::winit::platform::android::EventLoopBuilderExtAndroid;
			builder.with_android_app(app);
		})),
		renderer: eframe::Renderer::Wgpu,
		..Default::default()
	};
	crate::common(options,move|show|{
		if *show{
			println!("ime hide");
			let state=app2.text_input_state();
			println!("{}",&state.text);
			soft_input(app2.clone(),false);
			*show=false;
		}else{
			println!("ime show");
			soft_input(app2.clone(),true);
			*show=true;
		}
	});
}
fn soft_input(app:AndroidApp,show:bool){
	let jvm=unsafe{
		jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _).unwrap()
	};
	let activity = unsafe{
		jni::objects::JObject::from_raw(app.activity_as_ptr() as *mut _)
	};
	let mut env=jvm.attach_current_thread().unwrap();
	let class_ctxt = env.find_class("android/content/Context").unwrap();
	let ime = env
		.get_static_field(class_ctxt, "INPUT_METHOD_SERVICE", "Ljava/lang/String;")
		.unwrap();
	let ime_manager = env
		.call_method(
			&activity,
			"getSystemService",
			"(Ljava/lang/String;)Ljava/lang/Object;",
			&[ime.borrow()],
		)
		.unwrap()
		.l()
		.unwrap();

	let jni_window = env
		.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])
		.unwrap()
		.l()
		.unwrap();
	let view = env
		.call_method(jni_window, "getDecorView", "()Landroid/view/View;", &[])
		.unwrap()
		.l()
		.unwrap();
	/*
	let enabled_input_method_list=env.call_method(
		&ime_manager,
		"getEnabledInputMethodList",
		"()Ljava/util/List;",
		&[],
	).unwrap().l().unwrap();
	let size=env.call_method(&enabled_input_method_list,"size","()I",&[]).unwrap().i().unwrap();
	for i in 0..size{
		println!("get {}/{}",i,size);
		let info=env.call_method(
			&enabled_input_method_list,
			"get",
			"(I)Ljava/lang/Object;",
			&[i.into()],
		).unwrap().l().unwrap();
		let s=env.call_method(&info,"toString","()Ljava/lang/String;",&[]).unwrap().l().unwrap();
		let s=jni::objects::JString::from(s);
		let s=env.get_string(&s).unwrap();
		println!("getEnabledInputMethodList {}/{}",i,s.to_string_lossy());
	}
	*/

	if show{
		let result = env
		.call_method(
			&ime_manager,
			"showSoftInput",
			"(Landroid/view/View;I)Z",
			&[jni::objects::JValueGen::Object(&view), 0i32.into()],
		)
		.unwrap()
		.z()
		.unwrap();
		println!("show input: {}", result);
	}else{
		let window_token = env
		.call_method(view, "getWindowToken", "()Landroid/os/IBinder;", &[])
		.unwrap()
		.l()
		.unwrap();
		let result = env
			.call_method(
				&ime_manager,
				"hideSoftInputFromWindow",
				"(Landroid/os/IBinder;I)Z",
				&[jni::objects::JValueGen::Object(&window_token), 0i32.into()],
			)
			.unwrap()
			.z()
			.unwrap();
		println!("hide input: {}", result);
	}
}

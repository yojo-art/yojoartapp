fn main(){
	#[cfg(not(target_os="android"))]
	yojo_art_app::open()
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod diagnostics;
mod export;
mod import;
mod mesh;
mod platform;
mod renderer;
mod settings;

use std::{path::{Path,PathBuf},rc::Rc,sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}}};

use mesh::{SceneDocument,SceneMesh};
use renderer::{Renderer,SharedRenderState};
use settings::Settings;
use slint::{ComponentHandle,ModelRc,SharedString,VecModel};

slint::include_modules!();

fn main(){
    diagnostics::install();
    diagnostics::log("entering Rust main");
    if let Err(error)=run_app(){
        diagnostics::log(format!("fatal application error: {error}"));
        let message=format!("3D Viewer could not start.\n\n{error}\n\nDiagnostic log:\n{}",diagnostics::display_path());
        let _=rfd::MessageDialog::new().set_title("3D Viewer").set_description(message).set_level(rfd::MessageLevel::Error).show();
    }
}

fn run_app()->Result<(),Box<dyn std::error::Error>>{
    diagnostics::log("selecting Slint WGPU backend");
    let mut wgpu_settings=slint::wgpu_30::WGPUSettings::default();
    #[cfg(target_os="windows")]{wgpu_settings.backends=slint::wgpu_30::wgpu::Backends::DX12;wgpu_settings.power_preference=slint::wgpu_30::wgpu::PowerPreference::HighPerformance;}
    slint::BackendSelector::new().require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(wgpu_settings)).select()?;
    diagnostics::log("Slint WGPU backend selected");

    diagnostics::log("registering Windows file capabilities");
    let _=platform::register_file_capabilities();
    diagnostics::log("creating MainWindow");
    let app=MainWindow::new()?;
    diagnostics::log("MainWindow created");
    let settings=Arc::new(Mutex::new(Settings::load()));
    {let s=settings.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();app.set_theme_mode(s.theme_mode);app.set_filament_price(s.filament_price_per_kg);app.set_filament_density(s.filament_density_g_cm3);app.set_infill_percent(s.infill_percent);app.set_cad_linear_deflection(s.cad_linear_deflection_mm);app.set_cad_angular_deflection(s.cad_angular_deflection_deg);app.set_cad_relative_deflection(s.cad_relative_deflection);}

    let initial=SceneDocument::unit_cube();
    let shared=Arc::new(Mutex::new(SharedRenderState::new(initial)));
    {let state=shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());refresh_scene_ui(&app,&state,Some("Demo cube"));sync_view_ui(&app,&state);}
    setup_renderer(&app,shared.clone())?;
    diagnostics::log("rendering notifier installed");
    wire_callbacks(&app,shared.clone(),settings.clone());

    if let Some(path)=std::env::args_os().nth(1).map(PathBuf::from){load_into_app(&app,&shared,&path);}
    diagnostics::log("entering Slint event loop");
    app.run()?;
    diagnostics::log("Slint event loop returned normally");
    Ok(())
}

fn setup_renderer(app:&MainWindow,shared:Arc<Mutex<SharedRenderState>>)->Result<(),slint::SetRenderingNotifierError>{
    let weak=app.as_weak();let mut renderer:Option<Renderer>=None;let mut first_render_logged=false;
    app.window().set_rendering_notifier(move |state,api|match state{
        slint::RenderingState::RenderingSetup=>{
            diagnostics::log("WGPU RenderingSetup received");
            if let slint::GraphicsAPI::WGPU30{device,queue,..}=api{
                let gpu_error_count=Arc::new(AtomicUsize::new(0));
                let gpu_error_count_for_handler=gpu_error_count.clone();
                device.on_uncaptured_error(Arc::new(move |error|{
                    let index=gpu_error_count_for_handler.fetch_add(1,Ordering::Relaxed);
                    if index<8{diagnostics::log(format!("WGPU uncaptured error #{}: {error:?}",index+1));}
                    else if index==8{diagnostics::log("WGPU uncaptured errors: further repeated errors suppressed for this device session");}
                }));
                let info=device.adapter_info();
                diagnostics::log(format!("WGPU adapter: {} / {:?}",info.name,info.backend));
                if let Some(app)=weak.upgrade(){let backend=if info.backend==slint::wgpu_30::wgpu::Backend::Dx12{"DirectX 12".to_string()}else{format!("{:?}",info.backend)};app.set_renderer_text(format!("{backend} • WGPU 30").into());app.set_gpu_text(info.name.into());}
                let (initial_width,initial_height)=weak.upgrade().map(|app|(app.get_viewport_width_px().round().max(1.0) as u32,app.get_viewport_height_px().round().max(1.0) as u32)).unwrap_or((1280,800));
                {
                    let mut state=shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.set_viewport_aspect(initial_width as f32 / initial_height.max(1) as f32);
                    state.fit();
                    if let Some(app)=weak.upgrade(){sync_view_ui(&app,&state);}
                }
                let new_renderer=Renderer::new(device,queue,initial_width,initial_height);
                diagnostics::log(format!("initial viewport target: {initial_width}x{initial_height}"));
                match slint::Image::try_from(new_renderer.output_texture()){
                    Ok(image)=>{
                        if let Some(app)=weak.upgrade(){app.set_viewport_image(image);}
                        diagnostics::log("initial WGPU viewport texture imported into Slint");
                    }
                    Err(error)=>{
                        diagnostics::log(format!("initial WGPU texture import failed: {error:?}"));
                        if let Some(app)=weak.upgrade(){app.set_status_text(format!("Viewport texture import failed: {error:?}").into());}
                    }
                }
                renderer=Some(new_renderer);
                diagnostics::log("custom viewport renderer created");
            }else{
                diagnostics::log("RenderingSetup did not provide WGPU30 API");
            }
        }
        slint::RenderingState::BeforeRendering=>{
            if let(Some(r),Some(app))=(renderer.as_mut(),weak.upgrade()){
                let width=app.get_viewport_width_px().round().max(1.0) as u32;
                let height=app.get_viewport_height_px().round().max(1.0) as u32;
                shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).set_viewport_aspect(width as f32 / height.max(1) as f32);
                if let Some(texture)=r.ensure_targets(width,height){
                    match slint::Image::try_from(texture){
                        Ok(image)=>{app.set_viewport_image(image);app.window().request_redraw();diagnostics::log(format!("viewport target resized/imported: {width}x{height}; redraw queued"));}
                        Err(error)=>{
                            diagnostics::log(format!("WGPU resize texture import failed: {error:?}"));
                            app.set_status_text(format!("Viewport resize import failed: {error:?}").into());
                        }
                    }
                }
                r.render(&shared);
                if !first_render_logged{diagnostics::log(format!("first viewport frame submitted: {width}x{height}"));first_render_logged=true;}
            }
        }
        slint::RenderingState::RenderingTeardown=>{diagnostics::log("WGPU RenderingTeardown received");renderer=None},_=>{}
    })
}
fn request(app:&MainWindow){app.window().request_redraw();}

fn sync_view_ui(app:&MainWindow,state:&SharedRenderState){
    app.set_zoom_percent(state.zoom_percent());
    app.set_mode_2d(state.mode_2d);
    app.set_view_mode_text(SharedString::from(if state.mode_2d{"2D Orthographic"}else{"3D Perspective"}));
}

fn wire_callbacks(app:&MainWindow,shared:Arc<Mutex<SharedRenderState>>,settings:Arc<Mutex<Settings>>){
    let weak=app.as_weak();let sh=shared.clone();app.on_open_file(move||{if let Some(path)=rfd::FileDialog::new().add_filter("3D models",import::ALL_EXTENSIONS).pick_file(){if let Some(app)=weak.upgrade(){load_into_app(&app,&sh,&path);}}});

    let weak=app.as_weak();let sh=shared.clone();app.on_export_file(move||{
        let Some(app)=weak.upgrade()else{return;};
        let Some(path)=rfd::FileDialog::new().set_file_name("model.obj").add_filter("OBJ",&["obj"]).add_filter("STL",&["stl"]).add_filter("OFF",&["off"]).add_filter("PLY",&["ply"]).add_filter("glTF",&["gltf"]).add_filter("Binary glTF",&["glb"]).add_filter("dotbim",&["bim"]).save_file()else{return;};
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{let state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());export::save(&state.scene,&path)}));
        let message=match result{Ok(Ok(_))=>format!("Exported {}",path.display()),Ok(Err(e))=>format!("Export failed: {e}"),Err(_)=>{diagnostics::log(format!("Exporter panic was contained for {}",path.display()));"Export failed safely: exporter aborted while writing this file".to_string()}};
        app.set_status_text(SharedString::from(message));
    });

    macro_rules! camera_cb{($setter:ident,$body:expr)=>{{let weak=app.as_weak();let sh=shared.clone();app.$setter(move||{let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());$body(&mut state);if let Some(a)=weak.upgrade(){sync_view_ui(&a,&state);drop(state);request(&a);}});}}}

    let weak=app.as_weak();let sh=shared.clone();app.on_fit_view(move||{if let Some(a)=weak.upgrade(){let w=a.get_viewport_width_px().max(1.0);let h=a.get_viewport_height_px().max(1.0);let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.set_viewport_aspect(w/h);state.fit();sync_view_ui(&a,&state);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_fit_selected(move||{if let Some(a)=weak.upgrade(){let w=a.get_viewport_width_px().max(1.0);let h=a.get_viewport_height_px().max(1.0);let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.set_viewport_aspect(w/h);state.fit_selected();sync_view_ui(&a,&state);drop(state);request(&a);}});

    camera_cb!(on_rotate_left,|s:&mut SharedRenderState|s.rotate(-0.12,0.0));camera_cb!(on_rotate_right,|s:&mut SharedRenderState|s.rotate(0.12,0.0));camera_cb!(on_rotate_up,|s:&mut SharedRenderState|s.rotate(0.0,0.10));camera_cb!(on_rotate_down,|s:&mut SharedRenderState|s.rotate(0.0,-0.10));camera_cb!(on_zoom_in,|s:&mut SharedRenderState|s.zoom(0.88));camera_cb!(on_zoom_out,|s:&mut SharedRenderState|s.zoom(1.14));camera_cb!(on_front_view,|s:&mut SharedRenderState|s.front());camera_cb!(on_back_view,|s:&mut SharedRenderState|s.back());camera_cb!(on_left_view,|s:&mut SharedRenderState|s.left());camera_cb!(on_right_view,|s:&mut SharedRenderState|s.right());camera_cb!(on_top_view,|s:&mut SharedRenderState|s.top());camera_cb!(on_bottom_view,|s:&mut SharedRenderState|s.bottom());camera_cb!(on_iso_view,|s:&mut SharedRenderState|{s.set_2d_mode(false);s.iso();});

    let weak=app.as_weak();let sh=shared.clone();app.on_orbit_view(move|dx,dy|{let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.rotate(dx*0.008,-dy*0.008);if let Some(a)=weak.upgrade(){sync_view_ui(&a,&state);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_pan_view(move|dx,dy|{let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.pan(dx,dy);if let Some(a)=weak.upgrade(){sync_view_ui(&a,&state);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_wheel_zoom(move|delta|{let factor=(-delta*0.0015).exp().clamp(0.50,2.0);let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.zoom(factor);if let Some(a)=weak.upgrade(){sync_view_ui(&a,&state);drop(state);request(&a);}});

    let weak=app.as_weak();let sh=shared.clone();app.on_set_zoom_percent(move|percent|{let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.set_zoom_percent(percent);if let Some(a)=weak.upgrade(){sync_view_ui(&a,&state);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_toggle_2d(move|enabled|{let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.set_2d_mode(enabled);if let Some(a)=weak.upgrade(){sync_view_ui(&a,&state);a.set_status_text(if enabled{"2D orthographic mode • orbit locked • pan/zoom enabled".into()}else{"3D perspective mode".into()});drop(state);request(&a);}});

    let weak=app.as_weak();let sh=shared.clone();app.on_toggle_wireframe(move|value|{sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).wireframe=value;if let Some(a)=weak.upgrade(){request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_toggle_show_edges(move|value|{sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).show_edges=value;if let Some(a)=weak.upgrade(){request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_toggle_use_textures(move|value|{sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).use_textures=value;if let Some(a)=weak.upgrade(){request(&a);}});

    let weak=app.as_weak();let sh=shared.clone();app.on_select_mesh(move|index|{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.select(index.max(0) as usize);refresh_scene_ui(&a,&state,None);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_select_previous_mesh(move||{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());let len=state.scene.objects.len();if len>0{let next=if state.selected_index==0{len-1}else{state.selected_index-1};state.select(next);refresh_scene_ui(&a,&state,None);}drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_select_next_mesh(move||{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());let len=state.scene.objects.len();if len>0{let next=(state.selected_index+1)%len;state.select(next);refresh_scene_ui(&a,&state,None);}drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_toggle_mesh_visible(move|index|{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.toggle_visible(index.max(0) as usize);refresh_scene_ui(&a,&state,None);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_isolate_mesh(move|index|{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.isolate(index.max(0) as usize);state.fit_selected();refresh_scene_ui(&a,&state,None);sync_view_ui(&a,&state);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_show_all_meshes(move||{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());state.show_all();refresh_scene_ui(&a,&state,None);drop(state);request(&a);}});
    let weak=app.as_weak();let sh=shared.clone();app.on_pick_mesh(move|x,y,w,h|{if let Some(a)=weak.upgrade(){let mut state=sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner());if state.pick(x,y,w,h).is_some(){refresh_scene_ui(&a,&state,None);}drop(state);request(&a);}});

    let weak=app.as_weak();app.on_open_default_apps(move||{if let Err(e)=platform::open_default_apps(){if let Some(a)=weak.upgrade(){a.set_status_text(e.into());}}});
    let weak=app.as_weak();let st=settings.clone();app.on_save_settings(move||{if let Some(a)=weak.upgrade(){let mut s=st.lock().unwrap_or_else(|poisoned| poisoned.into_inner());s.theme_mode=a.get_theme_mode();s.filament_price_per_kg=a.get_filament_price();s.filament_density_g_cm3=a.get_filament_density();s.infill_percent=a.get_infill_percent();s.cad_linear_deflection_mm=a.get_cad_linear_deflection();s.cad_angular_deflection_deg=a.get_cad_angular_deflection();s.cad_relative_deflection=a.get_cad_relative_deflection();s.save();a.set_status_text("Settings saved • CAD tessellation applies on next CAD import".into());}});
    let weak=app.as_weak();let sh=shared.clone();app.on_recalc_print(move||{if let Some(a)=weak.upgrade(){update_print_estimate(&a,&sh.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).scene);}});
}

fn load_into_app(app:&MainWindow,shared:&Arc<Mutex<SharedRenderState>>,path:&Path){
    app.set_status_text(format!("Loading {}…",path.display()).into());
    app.set_import_backend_text(import::backend_status_for_path(path).into());
    let options=import::ImportOptions{
        cad_linear_deflection_mm:app.get_cad_linear_deflection(),
        cad_angular_deflection_deg:app.get_cad_angular_deflection(),
        cad_relative_deflection:app.get_cad_relative_deflection(),
    };
    let import_result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||import::load(path,&options)));
    match import_result{
        Ok(Ok(loaded))=>{
            let backend=loaded.backend.clone();
            {let mut state=shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());let w=app.get_viewport_width_px().max(1.0);let h=app.get_viewport_height_px().max(1.0);state.set_viewport_aspect(w/h);state.set_scene(loaded.scene);refresh_scene_ui(app,&state,path.file_name().and_then(|x|x.to_str()));sync_view_ui(app,&state);update_print_estimate(app,&state.scene);}
            let note={let state=shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());let textured=state.scene.objects.iter().filter(|o|o.mesh.has_renderable_texture()).count();let assets=state.scene.texture_asset_count();if textured>0{format!(" • {textured} textured object(s)")}else if assets>0{format!(" • {assets} texture asset(s) detected")}else{String::new()}};
            app.set_import_backend_text(backend.clone().into());
            app.set_status_text(format!("Ready • {backend}{note}").into());request(app);
        }
        Ok(Err(e))=>app.set_status_text(format!("Import failed: {e}").into()),
        Err(_)=>{diagnostics::log(format!("Importer panic was contained for {}",path.display()));app.set_status_text("Import failed safely: importer aborted while parsing this file".into());}
    }
}

fn refresh_scene_ui(app:&MainWindow,state:&SharedRenderState,file:Option<&str>){
    let scene=&state.scene;
    app.set_model_name(scene.name.clone().into());if let Some(file)=file{app.set_current_file(file.into());}
    app.set_mesh_count_text(scene.object_count().to_string().into());app.set_visible_count_text(scene.visible_object_count().to_string().into());app.set_total_triangles_text(scene.triangle_count().to_string().into());
    let dims=scene.dimensions();app.set_scene_dimensions_text(format!("{:.3} × {:.3} × {:.3}",dims.x,dims.y,dims.z).into());

    let rows:Vec<MeshItem>=scene.objects.iter().enumerate().map(|(i,o)|MeshItem{name:o.name.clone().into(),visible:o.visible,selected:i==state.selected_index,triangles:o.mesh.triangle_count().to_string().into(),material:o.mesh.material_name.clone().into()}).collect();
    app.set_mesh_items(ModelRc::from(Rc::new(VecModel::from(rows))));
    app.set_selected_mesh_index(state.selected_index as i32);

    if let Some(object)=scene.objects.get(state.selected_index){refresh_selected_ui(app,&object.mesh,&object.name);}else{clear_selected_ui(app);}
    update_print_estimate(app,scene);
}

fn refresh_selected_ui(app:&MainWindow,mesh:&SceneMesh,object_name:&str){
    let d=mesh.dimensions();app.set_selected_name_text(object_name.into());app.set_vertices_text(mesh.vertices.len().to_string().into());app.set_triangles_text(mesh.triangle_count().to_string().into());app.set_dimensions_text(format!("{:.3} × {:.3} × {:.3}",d.x,d.y,d.z).into());app.set_area_text(format!("{:.4}",mesh.surface_area()).into());app.set_volume_text(format!("{:.4}",mesh.volume()).into());app.set_materials_text(mesh.material_name.clone().into());
    let texture_status=if let Some(embedded)=&mesh.embedded_base_color{if mesh.has_texture_coordinates(){format!("Applied • {} (embedded)",embedded.name)}else{"Embedded texture detected, but this mesh has no UV coordinates".to_string()}}else if let Some(path)=&mesh.base_color_texture{if mesh.has_texture_coordinates(){if mesh.generated_uvs{format!("Applied • {} • generated STL box UVs",path.file_name().and_then(|v|v.to_str()).unwrap_or("texture"))}else{format!("Applied • {}",path.file_name().and_then(|v|v.to_str()).unwrap_or("texture"))}}else{"Detected, but this mesh has no UV coordinates".to_string()}}else if mesh.texture_asset_count()>0{if mesh.has_texture_coordinates(){format!("{} asset(s) found • no unambiguous base color",mesh.texture_asset_count())}else{format!("{} asset(s) found • no usable base-color map",mesh.texture_asset_count())}}else{"No local or embedded textures detected".to_string()};
    app.set_texture_status_text(texture_status.into());app.set_texture_assets_text(mesh.texture_asset_count().to_string().into());let texture_path=if let Some(embedded)=&mesh.embedded_base_color{format!("embedded://{}",embedded.name)}else{mesh.base_color_texture.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"—".to_string())};app.set_texture_path_text(texture_path.into());
}
fn clear_selected_ui(app:&MainWindow){app.set_selected_name_text("No selection".into());app.set_vertices_text("0".into());app.set_triangles_text("0".into());app.set_dimensions_text("—".into());app.set_area_text("0".into());app.set_volume_text("0".into());app.set_materials_text("—".into());app.set_texture_status_text("No selection".into());app.set_texture_assets_text("0".into());app.set_texture_path_text("—".into());}

fn update_print_estimate(app:&MainWindow,scene:&SceneDocument){
    let volume_mm3:f64=scene.objects.iter().filter(|o|o.visible).filter_map(|o|{let v=o.mesh.volume();(v.is_finite()&&v>=0.0).then_some(v)}).sum();
    let volume_cm3=(volume_mm3/1000.0).max(0.0);
    let infill=(app.get_infill_percent()/100.0).clamp(0.0,1.0) as f64;
    let density=(app.get_filament_density() as f64).clamp(0.01,100.0);
    let price=(app.get_filament_price() as f64).clamp(0.0,1_000_000.0);
    let material_fraction=(0.20+0.80*infill).clamp(0.0,1.0);
    let grams=(volume_cm3*density*material_fraction).max(0.0);
    let cost=(grams/1000.0*price).max(0.0);
    app.set_print_weight_text(format!("{grams:.2} g ({:.4} kg)",grams/1000.0).into());app.set_print_cost_text(format!("{cost:.2} lei").into());
}

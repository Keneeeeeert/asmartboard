use crate::assets::ICON;
use crate::passthrough_helper::PassthroughHelper;
use crate::render::RenderState;
use crate::UserEvent;
#[cfg(feature = "startup_animation")]
use crate::state::StartupAnimation;
use crate::state::{
    AppState, CanvasObject, CanvasObjectOps, CanvasTool, InsertTab, PointerInteraction,
    PointerState,
};
use crate::ui;
use crate::utils;
use crate::utils::stroke::{brush_stroke_add_point, brush_stroke_end, brush_stroke_start};
use crate::utils::ui::{apply_theme_mode_and_canvas_color, apply_window_mode};
use core::f32;
use egui::{Pos2, Vec2};
use egui_wgpu::{ScreenDescriptor, wgpu};
use image::GenericImageView;
use std::sync::Arc;
use wgpu::{
    BackendOptions, CurrentSurfaceTexture, InstanceDescriptor, TexelCopyBufferInfo,
    TexelCopyBufferLayout,
};
use tray_icon::TrayIconBuilder;
use wgpu::{InstanceFlags, TexelCopyTextureInfo};
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, Touch, TouchPhase, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

#[derive(Clone, Copy)]
pub enum FileDialogAction {
    Save,
    Load,
}

pub struct App {
    pub gpu_instance: wgpu::Instance,
    pub render_state: Option<RenderState>,
    pub window: Option<Arc<Window>>,
    pub state: AppState,
    pub helper_window: Option<PassthroughHelper>,
    modifiers: winit::keyboard::ModifiersState,
}

impl App {
    pub fn new() -> Self {
        let mut state = AppState::default();
        let gpu_instance = wgpu::Instance::new(InstanceDescriptor {
            backends: state.persistent.graphics_api.to_backends(),
            flags: InstanceFlags::empty(),
            memory_budget_thresholds: Default::default(),
            backend_options: {
                let mut options = BackendOptions::default();
                options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
                options
            },
            display: None,
        });

        if !state.persistent.show_welcome_window_on_start {
            state.show_welcome_window = false
        }

        #[cfg(feature = "startup_animation")]
        if state.persistent.show_startup_animation {
            state.startup_animation = Some(StartupAnimation::new(
                30.0,
                crate::assets::STARTUP_FRAMES,
                crate::assets::STARTUP_AUDIO,
            ));
        }

        Self {
            gpu_instance,
            render_state: None,
            window: None,
            state,
            helper_window: None,
            modifiers: winit::keyboard::ModifiersState::default(),
        }
    }

    pub async fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        // icon
        let icon = image::load_from_memory(ICON).expect("invalid icon data");
        let rgba = icon.to_rgba8().to_vec();
        let (width, height) = icon.dimensions();
        let winit_icon =
            Some(winit::window::Icon::from_rgba(rgba.clone(), width, height).expect("invalid icon data"));

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("erh_smartboard")
                        .with_transparent(true)
                        .with_window_icon({
                            #[cfg(target_os = "windows")]
                            {
                                winit_icon.clone()
                            }
                            #[cfg(not(target_os = "windows"))]
                            winit_icon
                        }),
                )
                .unwrap(),
        );

        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowExtWindows;
            window.set_taskbar_icon(winit_icon);
        }

        // prepare exclusive fullscreen video modes
        let monitor = window
            .current_monitor()
            .or_else(|| window.primary_monitor())
            .or_else(|| window.available_monitors().next());
        if let Some(monitor) = monitor {
            self.state.fullscreen_video_modes = monitor.video_modes().collect();
        } else {
            eprintln!(
                "warning: failed to get monitor, exclusive fullscreen mode will be unavailable"
            )
        }

        // window mode
        apply_window_mode(&mut self.state, &window);

        // 创建托盘图标
        let tray = TrayIconBuilder::new()
            .with_icon(tray_icon::Icon::from_rgba(rgba, width, height).expect("invalid icon data"))
            .with_tooltip("erh_smartboard")
            .build()
            .unwrap();
        let _ = tray.set_visible(false);
        self.state.tray = Some(tray);

        #[cfg(target_os = "windows")]
        unsafe {
            if let Err(err) = utils::windows::enable_premultiplied_alpha(
                utils::windows::winit_window_to_hwnd(&window).unwrap(),
            ) {
                eprintln!(
                    "
error: failed to enable premultiplied alpha for window: {:?}
       passthrough mode might not work or app might crash",
                    err
                );
            }
        };

        // prepare renderer
        let size = window.inner_size();
        let initial_width = size.width;
        let initial_height = size.height;

        let surface = self
            .gpu_instance
            .create_surface(window.clone())
            .expect("failed to create surface");

        let state = RenderState::new(
            &self.gpu_instance,
            surface,
            &window,
            initial_width,
            initial_height,
            self.state.persistent.optimization_policy,
            self.state.persistent.present_mode,
        )
        .await;

        self.state.active_backend = Some(state.device.adapter_info().backend);

        let ctx = state.egui_renderer.context();

        // colors
        apply_theme_mode_and_canvas_color(
            ctx,
            self.state.persistent.theme_mode,
            self.state.persistent.canvas_color,
        );

        // first draw
        window.request_redraw();

        self.window.get_or_insert(window);
        self.render_state.get_or_insert(state);
    }

    fn exit(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(err) = self.state.persistent.save_to_file() {
            eprintln!("failed to save settings: {}", err);
        }
        event_loop.exit();
    }

    fn handle_resized(&mut self, width: u32, height: u32) {
        self.render_state
            .as_mut()
            .unwrap()
            .resize_surface(width, height);
    }

    #[cfg_attr(feature = "profiling", profiling::function)]
    fn handle_redraw(&mut self) {
        #[cfg(feature = "profiling")]
        profiling::scope!("handle_redraw::setup");

        let render_state = self.render_state.as_mut().unwrap();

        if self.state.present_mode_changed {
            render_state.set_present_mode(self.state.persistent.present_mode);
            self.state.present_mode_changed = false;
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [
                render_state.surface_config.width,
                render_state.surface_config.height,
            ],
            pixels_per_point: self.window.as_ref().unwrap().scale_factor() as f32
                * render_state.scale_factor,
        };

        let surface_texture = render_state.surface.get_current_texture();

        let surface_texture = match surface_texture {
            CurrentSurfaceTexture::Success(surface) => surface,
            CurrentSurfaceTexture::Suboptimal(surface) => {
                println!("warning: wgpu surface suboptimal");
                surface
            }
            val => {
                println!("warning: wgpu surface {:?}", val);
                return;
            }
        };

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = render_state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        let window = self.window.as_ref().unwrap();

        render_state.egui_renderer.begin_frame(window);

        // access this value in next redraw before ui to ensure that all ui has become invisible
        let screenshot_path = self.state.screenshot_path.clone();

        // fixes a borrow checker error
        let ctx = &(render_state.egui_renderer.context().clone());

        // --- ui ---
        {
            #[cfg(feature = "profiling")]
            profiling::scope!("handle_redraw::ui");

            #[cfg(feature = "startup_animation")]
            if let Some(anim) = &mut self.state.startup_animation {
                if !anim.is_finished() {
                    anim.update(ctx);
                    anim.draw_fullscreen(ctx);
                    ctx.request_repaint(); // ensure smooth playback
                }
            }

            self.state.toasts.show(ctx);

            #[cfg(feature = "profiling")]
            puffin_egui::profiler_window(ctx);

            if self.state.current_tool != CanvasTool::Passthrough
                && self.state.screenshot_path.is_none()
            {
                if self.state.show_welcome_window {
                    ui::ui_welcome(&mut self.state, ctx);
                }

                ui::ui_toolbar(&mut self.state, ctx, window);

                ui::ui_pages_nav(&mut self.state, ctx);

                if self.state.show_page_management_window {
                    ui::ui_pages_manager(&mut self.state, ctx);
                }
            }

            ui::ui_canvas(&mut self.state, ctx);
        };
        // --- end ui

        // egui render pass
        {
            #[cfg(feature = "profiling")]
            profiling::scope!("handle_redraw::render_pass");

            render_state.egui_renderer.end_frame_and_draw(
                &render_state.device,
                &render_state.queue,
                &mut encoder,
                window,
                &surface_view,
                screen_descriptor,
            );
        }

        // submit & present texture
        if let Some(path) = screenshot_path {
            #[cfg(feature = "profiling")]
            profiling::scope!("handle_redraw::screenshot");

            let width = render_state.surface_config.width;
            let height = render_state.surface_config.height;

            let bytes_per_pixel = 4;
            let unpadded_bytes_per_row = width * bytes_per_pixel;

            // wgpu requires 256-byte alignment
            const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(ALIGN) * ALIGN;

            let buffer_size = (padded_bytes_per_row * height) as u64;

            let output_buffer = render_state.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("screenshot buffer"),
                size: buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &surface_texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &output_buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            render_state.queue.submit(Some(encoder.finish()));

            let buffer_slice = output_buffer.slice(..);

            buffer_slice.map_async(wgpu::MapMode::Read, |_| {});

            // ensure gpu work is done
            let _ = render_state.device.poll(wgpu::wgt::PollType::Wait {
                submission_index: None,
                timeout: None,
            });

            let data = buffer_slice.get_mapped_range();

            let mut pixels = vec![0u8; (width * height * 4) as usize];

            for y in 0..height as usize {
                let src_offset = y * padded_bytes_per_row as usize;
                let dst_offset = y * unpadded_bytes_per_row as usize;

                pixels[dst_offset..dst_offset + unpadded_bytes_per_row as usize].copy_from_slice(
                    &data[src_offset..src_offset + unpadded_bytes_per_row as usize],
                );
            }

            // pixels
            //     .chunks_exact(width as usize * 4)
            //     .collect::<Vec<_>>()
            //     .into_iter()
            //     .rev()
            //     .flatten()
            //     .copied()
            //     .collect::<Vec<u8>>();

            for chunk in pixels.chunks_exact_mut(4) {
                chunk.swap(0, 2); // B ↔ R
            }

            match image::save_buffer(path, &pixels, width, height, image::ColorType::Rgba8) {
                Ok(_) => {
                    self.state.toasts.success("成功导出为图片!");
                }
                Err(err) => {
                    self.state.toasts.error(format!("画布导出失败: {}!", err));
                }
            }

            drop(data);
            output_buffer.unmap();

            self.state.screenshot_path = None;
        } else {
            render_state.queue.submit(Some(encoder.finish()));
        }

        {
            #[cfg(feature = "profiling")]
            profiling::scope!("handle_redraw::gc");

            self.state.canvas.objects.retain(|obj| {
                if let CanvasObject::Image(img) = obj {
                    !img.marked_for_deletion
                } else {
                    true
                }
            });
        }

        surface_texture.present();

        // update window passthrough state once if disabled
        if self.state.overlay_mode_changed && !self.state.is_overlay_mode {
            let _ = window.set_cursor_hittest(true);
            self.state.overlay_mode_changed = false;
        }

        // update window passthrough state every frame if enabled
        if self.state.is_overlay_mode {
            if self.state.current_tool == CanvasTool::Passthrough {
                let _ = window.set_cursor_hittest(false);
            } else {
                let _ = window.set_cursor_hittest(true);
            }
        }

        if self.state.persistent.show_fps {
            _ = self.state.fps_counter.update();
        }

        #[cfg(feature = "profiling")]
        profiling::finish_frame!();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        pollster::block_on(self.create_window(event_loop));
    }

    // redraw if egui requests repaint
    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.state.should_quit {
            return;
        }

        self.request_helper_repaint_if_needed();

        if let Some(render_state) = self.render_state.as_ref()
            && render_state.egui_renderer.context().has_requested_repaint()
        {
            self.window.as_ref().unwrap().request_redraw();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TrayIconEvent(event) => {
                if let tray_icon::TrayIconEvent::Click { .. } = event {
                    let window = self.window.as_ref().unwrap();
                    window.set_visible(true);
                    window.focus_window();
                    if let Some(tray) = &self.state.tray {
                        let _ = tray.set_visible(false);
                    }
                    window.request_redraw();
                }
            }
            UserEvent::FileDialogResult {
                path: Some(path),
                action: FileDialogAction::Save,
                page_index,
            } => {
                let canvas = match page_index {
                    Some(i) => &self.state.pages[i].canvas,
                    None => &self.state.canvas,
                };
                match canvas.save_to_file(&path) {
                    Ok(_) => {
                        self.state.toasts.success("已保存");
                    }
                    Err(e) => {
                        self.state.toasts.error(format!("保存失败: {e}"));
                    }
                }
                self.window.as_ref().unwrap().request_redraw();
            }
            UserEvent::FileDialogResult {
                path: Some(path),
                action: FileDialogAction::Load,
                ..
            } => {
                let canvas = crate::state::CanvasState::load_from_file(&path);
                match canvas {
                    Ok(canvas) => {
                        let page = crate::state::PageState {
                            canvas,
                            history: crate::state::History::default(),
                        };
                        let idx = self.state.pages.len();
                        self.state.pages.push(page.clone());
                        self.state.current_page = idx;
                        self.state.canvas = page.canvas;
                        self.state.history = page.history;
                        crate::utils::ui::clear_interaction_state(&mut self.state);
                        self.state.show_welcome_window = false;
                        self.state.toasts.success("已加载");
                    }
                    Err(e) => {
                        self.state.toasts.error(format!("加载失败: {e}"));
                    }
                }
                self.window.as_ref().unwrap().request_redraw();
            }
            UserEvent::FileDialogResult { .. } => {
                // user cancelled
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Dispatch to helper window if this event is for it
        if self.is_event_for_helper(window_id) {
            self.handle_helper_window_event(event_loop, event);
            return;
        }

        if self.state.should_quit {
            println!("quit button was pressed; exiting");
            self.exit(event_loop);
            return;
        }

        // redraw only on input
        // don't pass RedrawRequested to egui's input handler,
        // it's not input and would make egui request a repaint, causing an infinite redraw loop
        if self.state.persistent.force_redraw_every_frame
            || !matches!(event, WindowEvent::RedrawRequested)
        {
            let egui_needs_repaint = self
                .render_state
                .as_mut()
                .unwrap()
                .egui_renderer
                .handle_input(self.window.as_ref().unwrap(), &event);

            if self.state.persistent.force_redraw_every_frame || egui_needs_repaint {
                self.window.as_ref().unwrap().request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                self.exit(event_loop);
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: winit::event::ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                let ctx = self.render_state.as_ref().unwrap().egui_renderer.context();
                if ctx.egui_wants_keyboard_input() {
                    // egui is handling input (e.g. text field), skip shortcuts
                } else {
                    match logical_key {
                        Key::Named(NamedKey::Escape) => self.exit(event_loop),
                        Key::Named(NamedKey::Delete) => {
                            if let Some(idx) = self.state.selected_object_index
                                && idx < self.state.canvas.objects.len()
                            {
                                let obj = self.state.canvas.objects.remove(idx);
                                self.state.history.save_remove_object(idx, obj);
                                self.state.selected_object_index = None;
                                self.window.as_ref().unwrap().request_redraw();
                            }
                        }
                        Key::Character(ref ch) => {
                            let ctrl = self.modifiers.control_key();
                            match ch.as_str() {
                                "z" if ctrl => {
                                    self.state.selected_object_index = None;
                                    self.state.history.undo(&mut self.state.canvas);
                                    self.window.as_ref().unwrap().request_redraw();
                                }
                                "y" if ctrl => {
                                    self.state.selected_object_index = None;
                                    self.state.history.redo(&mut self.state.canvas);
                                    self.window.as_ref().unwrap().request_redraw();
                                }
                                "s" if ctrl => {
                                    let proxy = crate::EVENT_PROXY.get().unwrap().clone();
                                    std::thread::spawn(move || {
                                        let path = rfd::FileDialog::new()
                                            .add_filter("画布文件", &["sb"])
                                            .set_file_name("canvas.sb")
                                            .save_file();
                                        let _ = proxy.send_event(UserEvent::FileDialogResult {
                                            path,
                                            action: FileDialogAction::Save,
                                            page_index: None,
                                        });
                                    });
                                }
                                "b" if !ctrl => {
                                    self.state.current_tool = CanvasTool::Brush;
                                    self.window.as_ref().unwrap().request_redraw();
                                }
                                "v" if !ctrl => {
                                    self.state.current_tool = CanvasTool::Select;
                                    self.window.as_ref().unwrap().request_redraw();
                                }
                                "e" if !ctrl => {
                                    self.state.current_tool = CanvasTool::ObjectEraser;
                                    self.window.as_ref().unwrap().request_redraw();
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.handle_redraw();
                self.manage_passthrough_helper(event_loop);
            }
            WindowEvent::Resized(new_size) if new_size.width > 0 && new_size.height > 0 => {
                self.handle_resized(new_size.width, new_size.height);
                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::Touch(Touch {
                phase,
                location,
                id,
                ..
            }) => {
                // Convert touch location to logical coordinates (screen space)
                let window = self.window.as_ref().unwrap();
                let scale_factor = window.scale_factor() as f32;
                let screen_pos = Pos2::new(
                    location.x as f32 / scale_factor,
                    location.y as f32 / scale_factor,
                );
                let pos = screen_pos + self.state.view_offset;

                match phase {
                    TouchPhase::Started => match self.state.current_tool {
                        CanvasTool::Pan => {
                            self.state.pointers.insert(
                                id,
                                PointerState {
                                    id,
                                    pos,
                                    prev_pos: None,
                                    interaction: PointerInteraction::Panning {
                                        last_pos: screen_pos,
                                    },
                                },
                            );
                        }
                        CanvasTool::Brush => {
                            brush_stroke_start(&mut self.state, id, pos);
                        }
                        CanvasTool::Select
                            if !self.state.pointers.values().any(|p| {
                                matches!(p.interaction, PointerInteraction::Selecting { .. })
                            }) =>
                        {
                            // Hit-test objects (last to first for z-order)
                            for (i, object) in self.state.canvas.objects.iter().enumerate().rev() {
                                if object.bounding_box().contains(pos) {
                                    self.state.selected_object_index = Some(i);
                                    break;
                                }
                            }

                            let (dragged_handle, drag_original_transform) = if let Some(idx) =
                                self.state.selected_object_index
                                && idx < self.state.canvas.objects.len()
                            {
                                let object = &self.state.canvas.objects[idx];
                                let bbox = object.bounding_box();
                                let handle = utils::get_transform_handle_at_pos(bbox, pos);
                                let transform = handle.is_some().then(|| object.get_transform());
                                (handle, transform)
                            } else {
                                (None, None)
                            };

                            self.state.pointers.insert(
                                id,
                                PointerState {
                                    id,
                                    pos,
                                    prev_pos: None,
                                    interaction: PointerInteraction::Selecting {
                                        drag_start: pos,
                                        dragged_handle,
                                        drag_original_transform,
                                        drag_accumulated_delta: Vec2::ZERO,
                                    },
                                },
                            );
                        }
                        CanvasTool::ObjectEraser | CanvasTool::PixelEraser => {
                            self.state.pointers.insert(
                                id,
                                PointerState {
                                    id,
                                    pos,
                                    prev_pos: None,
                                    interaction: PointerInteraction::Erasing,
                                },
                            );
                        }
                        CanvasTool::Insert
                            if self.state.current_insert_tab == InsertTab::Shape
                                && self.state.selected_shape_type.is_some() =>
                        {
                            let shape_type = self.state.selected_shape_type.unwrap();
                            self.state.pointers.insert(
                                id,
                                PointerState {
                                    id,
                                    pos,
                                    prev_pos: None,
                                    interaction: PointerInteraction::ShapeInsert {
                                        start_pos: pos,
                                        shape_type,
                                    },
                                },
                            );
                        }
                        _ => {}
                    },
                    TouchPhase::Moved => match self.state.current_tool {
                        CanvasTool::Pan => {
                            if let Some(pointer) = self.state.pointers.get_mut(&id) {
                                if let PointerInteraction::Panning { ref mut last_pos } =
                                    pointer.interaction
                                {
                                    let delta = screen_pos - *last_pos;
                                    self.state.view_offset -= delta;
                                    *last_pos = screen_pos;
                                }
                                pointer.pos = pos;
                            }
                        }
                        CanvasTool::Brush => {
                            brush_stroke_add_point(&mut self.state, id, pos, false);
                        }
                        CanvasTool::Select => {
                            if let Some(pointer) = self.state.pointers.get_mut(&id) {
                                pointer.pos = pos;

                                if let PointerInteraction::Selecting {
                                    ref mut drag_start,
                                    dragged_handle,
                                    ref mut drag_accumulated_delta,
                                    ..
                                } = pointer.interaction
                                {
                                    let delta = pos - *drag_start;

                                    if let Some(idx) = self.state.selected_object_index
                                        && idx < self.state.canvas.objects.len()
                                    {
                                        if let Some(handle) = dragged_handle {
                                            if let Some(object) =
                                                self.state.canvas.objects.get_mut(idx)
                                            {
                                                object.transform(handle, delta, *drag_start, pos);
                                            }
                                        } else {
                                            if let Some(object) =
                                                self.state.canvas.objects.get_mut(idx)
                                            {
                                                CanvasObject::move_object(object, delta);
                                            }
                                            *drag_accumulated_delta += delta;
                                        }
                                    }

                                    *drag_start = pos;
                                }
                            }
                        }
                        CanvasTool::ObjectEraser | CanvasTool::PixelEraser => {
                            if let Some(pointer) = self.state.pointers.get_mut(&id) {
                                pointer.pos = pos;
                            }
                        }
                        CanvasTool::Insert => {
                            if let Some(pointer) = self.state.pointers.get_mut(&id) {
                                pointer.pos = pos;
                            }
                        }
                        _ => {}
                    },
                    TouchPhase::Ended | TouchPhase::Cancelled => match self.state.current_tool {
                        CanvasTool::Pan => {
                            self.state.pointers.remove(&id);
                        }
                        CanvasTool::Brush => {
                            brush_stroke_end(&mut self.state, id);
                        }
                        CanvasTool::Select => {
                            if let Some(pointer) = self.state.pointers.get(&id)
                                && let PointerInteraction::Selecting {
                                    drag_accumulated_delta,
                                    drag_original_transform,
                                    ..
                                } = &pointer.interaction
                            {
                                if let Some(sel_idx) = self.state.selected_object_index
                                    && *drag_accumulated_delta != Vec2::ZERO
                                {
                                    self.state.history.save_move_object(
                                        sel_idx,
                                        -*drag_accumulated_delta,
                                        *drag_accumulated_delta,
                                    );
                                }
                                if let Some(original) = drag_original_transform.clone()
                                    && let Some(sel_idx) = self.state.selected_object_index
                                    && sel_idx < self.state.canvas.objects.len()
                                {
                                    let new_transform =
                                        self.state.canvas.objects[sel_idx].get_transform();
                                    self.state.history.save_transform_object(
                                        sel_idx,
                                        original,
                                        new_transform,
                                    );
                                }
                            }
                            self.state.pointers.remove(&id);
                        }
                        CanvasTool::ObjectEraser | CanvasTool::PixelEraser => {
                            self.state.pointers.remove(&id);
                        }
                        CanvasTool::Insert => {
                            if let Some(pointer) = self.state.pointers.remove(&id)
                                && let PointerInteraction::ShapeInsert {
                                    start_pos,
                                    shape_type,
                                } = pointer.interaction
                            {
                                let end_pos = pointer.pos;
                                crate::utils::ui::create_shape_object(
                                    &mut self.state,
                                    shape_type,
                                    start_pos,
                                    end_pos,
                                );
                            }
                        }
                        _ => {}
                    },
                }

                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::CursorMoved {
                device_id: _,
                position,
            } => {
                self.state.cursor_position = position;
                self.window.as_ref().unwrap().request_redraw();
            }
            _ => (),
        }
    }
}

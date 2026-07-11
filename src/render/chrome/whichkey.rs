//! WHICH-KEY PANEL chrome — the summoned bottom-left float card teaching a prefix's
//! follow-up keys (faint key column beside muted command names). Its row setter,
//! sidecar report, and shape/upload. Carved out of `chrome.rs` verbatim, no
//! behaviour change. See [`super`].

use super::*;

impl TextPipeline {
    // ===== WHICH-KEY PANEL ================================================

    /// Set (or clear) the WHICH-KEY panel's rows: `Some(rows)` summons the panel with
    /// those `(key, command-name)` continuations, `None` puts it down. The App calls
    /// this on the prefix PAUSE (summon) and the instant the chord resolves/aborts
    /// (dismiss); the headless `--whichkey` capture sets it once. Idempotent — the
    /// rows only feed the next `prepare_whichkey`.
    pub fn set_whichkey(&mut self, rows: Option<Vec<(String, String)>>) {
        self.whichkey_rows = rows;
    }

    /// The which-key panel's rows for the sidecar / tests, or `None` when it is down —
    /// so a headless assertion can confirm the summoned continuation list without
    /// eyeballing pixels. Clones the small row list.
    pub fn whichkey_report(&self) -> Option<Vec<(String, String)>> {
        self.whichkey_rows.clone()
    }

    /// Shape + upload the summoned WHICH-KEY hint panel this frame: a calm bottom-left
    /// float card listing the prefix's follow-up keys, each a FAINT key label in a
    /// left column beside its MUTED command name (recessive ink — NO amber, which is
    /// the caret's alone, DESIGN §3). Parked (nothing drawn) unless `whichkey_rows` is
    /// `Some`, so a default frame stays byte-identical. Button-free: it TEACHES the
    /// keys, it is not clickable.
    pub(in crate::render) fn prepare_whichkey(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> anyhow::Result<()> {
        let bounds = TextBounds { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let faint = theme::faint().to_glyphon();
        let muted = theme::muted().to_glyphon();
        let m = self.metrics;

        // DOWN: park the card elevation + the text off-screen (byte-identical default).
        let Some(rows) = self.whichkey_rows.clone() else {
            set_float_quads(
                &mut self.wk_shadow,
                &mut self.wk_border,
                &mut self.wk_card,
                device,
                queue,
                width,
                height,
                None,
                true,
            );
            self.wk_buffer
                .set_size(&mut self.font_system, Some(1.0), Some(m.line_height));
            self.wk_buffer.set_text(
                &mut self.font_system,
                "",
                &panel_attrs().color(muted),
                Shaping::Advanced,
                None,
            );
            self.wk_buffer.shape_until_scroll(&mut self.font_system, false);
            let area = TextArea {
                buffer: &self.wk_buffer,
                left: 0.0,
                top: -1000.0,
                scale: 1.0,
                bounds,
                default_color: muted,
                custom_glyphs: &[],
            };
            self.wk_renderer
                .prepare(
                    device,
                    queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    [area],
                    &mut self.swash_cache,
                )
                .map_err(|e| anyhow::anyhow!("glyphon whichkey prepare failed: {e:?}"))?;
            return Ok(());
        };

        // A quiet HEADER (the prefix) over the continuation rows. The key column is
        // space-padded to one width so the names line up (proportional-font alignment is
        // approximate but calm — the same space-padding the find panel / gutter use).
        let key_w = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
        // Owned line strings + a role tag: 0 = header (faint), 1 = key (faint),
        // 2 = name (muted). Each row is TWO spans (padded key, then name + newline).
        let mut owned: Vec<(String, u8)> = Vec::with_capacity(rows.len() * 2 + 1);
        owned.push((format!("{PREFIX_HEADER}\n"), 0));
        for (key, name) in &rows {
            // Right-pad the key to `key_w` then a two-space gutter before the name.
            let pad = key_w.saturating_sub(key.chars().count());
            owned.push((format!("{key}{}  ", " ".repeat(pad)), 1));
            owned.push((format!("{name}\n"), 2));
        }
        let base = panel_attrs();
        let body = GlyphMetrics::new(m.font_size, m.line_height);
        let spans: Vec<(&str, Attrs)> = owned
            .iter()
            .map(|(s, role)| {
                let attrs = match role {
                    0 | 1 => base.clone().color(faint).metrics(body),
                    _ => base.clone().color(muted).metrics(body),
                };
                (s.as_str(), attrs)
            })
            .collect();

        self.wk_buffer
            .set_size(&mut self.font_system, Some(width as f32), Some(height as f32));
        let default_attrs = base.clone().color(muted).metrics(body);
        self.wk_buffer.set_rich_text(
            &mut self.font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        self.wk_buffer.shape_until_scroll(&mut self.font_system, false);

        // Measure the shaped block, then plant a padded card in the BOTTOM-LEFT corner
        // (clear of the centered writing column, so it never covers where you type).
        let mut block_h = 0.0_f32;
        let mut block_w = 0.0_f32;
        for run in self.wk_buffer.layout_runs() {
            block_h = block_h.max(run.line_top + run.line_height);
            block_w = block_w.max(run.line_w);
        }
        let pad_x = m.char_width * 2.0;
        let pad_y = m.line_height * 0.6;
        let margin = 24.0_f32;
        let card_w = block_w + pad_x * 2.0;
        let card_h = block_h + pad_y * 2.0;
        let card_x = margin;
        let card_y = (height as f32 - margin - card_h).max(margin);
        set_float_quads(
            &mut self.wk_shadow,
            &mut self.wk_border,
            &mut self.wk_card,
            device,
            queue,
            width,
            height,
            Some([card_x, card_y, card_w, card_h]),
            true,
        );
        let area = TextArea {
            buffer: &self.wk_buffer,
            left: card_x + pad_x,
            top: card_y + pad_y,
            scale: 1.0,
            bounds,
            default_color: muted,
            custom_glyphs: &[],
        };
        self.wk_renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash_cache,
            )
            .map_err(|e| anyhow::anyhow!("glyphon whichkey prepare failed: {e:?}"))?;
        Ok(())
    }
}

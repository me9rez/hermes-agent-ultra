//! Render WhatsApp pairing QR data as Unicode block art in the terminal.

use std::io::{self, Write};
use std::time::Duration;

/// Print a scannable QR code for WhatsApp Linked Devices pairing.
pub fn print_pairing_qr(data: &str, timeout: Duration) {
    println!();
    println!(
        "Scan with WhatsApp → Linked Devices (valid ~{}s):",
        timeout.as_secs()
    );
    println!();
    render_qr_to_terminal(data);
    let _ = io::stdout().flush();
}

/// Render QR payload as Unicode block art (same style as Weixin / legacy Baileys CLI).
pub fn render_qr_to_terminal(data: &str) {
    let code = match qrcode::QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("QR render failed: {e}");
            eprintln!("Raw pairing data (scan via external QR tool if needed):");
            eprintln!("{data}");
            return;
        }
    };
    let side = code.width() as usize;
    let modules = code.to_colors();
    let padded = side + 8;
    let is_dark = |r: usize, c: usize| modules[r * side + c] == qrcode::Color::Dark;
    let mut row = 0usize;
    while row < padded {
        let mut line = String::new();
        for col in 0..padded {
            let qr_row = row.wrapping_sub(4);
            let qr_col = col.wrapping_sub(4);
            let top = qr_row < side && qr_col < side && is_dark(qr_row, qr_col);
            let qr_row2 = (row + 1).wrapping_sub(4);
            let bottom = qr_row2 < side && qr_col < side && is_dark(qr_row2, qr_col);
            line.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        println!("  {line}");
        row += 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_qr_does_not_panic() {
        render_qr_to_terminal("test-pairing-payload");
    }
}

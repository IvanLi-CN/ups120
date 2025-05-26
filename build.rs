fn main() {
    // Set default values if environment variables are not set
    let usb_vid = std::env::var("USB_VID").unwrap_or_else(|_| "0x1209".to_string());
    let usb_pid = std::env::var("USB_PID").unwrap_or_else(|_| "0x0002".to_string());
    let webusb_landing_url = std::env::var("WEBUSB_LANDING_URL").unwrap_or_else(|_| "http://localhost:25057".to_string());

    // Print cargo:rustc-env directives to make these available at compile time
    println!("cargo:rustc-env=USB_VID={}", usb_vid);
    println!("cargo:rustc-env=USB_PID={}", usb_pid);
    println!("cargo:rustc-env=WEBUSB_LANDING_URL={}", webusb_landing_url);

    // Rerun if any of these environment variables change
    println!("cargo:rerun-if-env-changed=USB_VID");
    println!("cargo:rerun-if-env-changed=USB_PID");
    println!("cargo:rerun-if-env-changed=WEBUSB_LANDING_URL");
}
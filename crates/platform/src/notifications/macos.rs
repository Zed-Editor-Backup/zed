use anyhow::Result;
use cocoa::base::nil;
use cocoa::foundation::NSString;
use gpui::AppContext;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

pub fn show_notification(title: &str, body: &str, _: &impl AppContext) -> Result<()> {
    unsafe {
        let notification_center: *mut Object = msg_send![
            class!(NSUserNotificationCenter),
            defaultUserNotificationCenter
        ];
        let notification: *mut Object = msg_send![class!(NSUserNotification), new];

        let title_str = NSString::alloc(nil).init_str(title);
        let body_str = NSString::alloc(nil).init_str(body);

        let _: () = msg_send![notification, setTitle:title_str];
        let _: () = msg_send![notification, setInformativeText:body_str];

        let _: () = msg_send![notification_center, deliverNotification:notification];
    }
    Ok(())
}

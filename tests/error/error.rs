
// pub struct SyscallContext;

// impl Signal for SyscallContext {
//     fn kill(&self, _caller: Caller, pid: isize, signum: u8) -> isize {
//         if let Some(target_task) =
//             unsafe { PROCESSOR.get_task(ProcId::from_usize(pid as usize)) }
//         {
//             if let Ok(signal_no) = SignalNo::try_from(signum) {
//                 if signal_no != SignalNo::ERR {
//                     target_task.signal.add_signal(signal_no);
//                     return 0;
//                 }
//             }
//         }
//         -1
//     }

//     fn sigaction(
//         &self,
//         _caller: Caller,
//         signum: u8,
//         action: usize,
//         old_action: usize,
//     ) -> isize {
//         if signum as usize > signal::MAX_SIG {
//             return -1;
//         }
//         let current = unsafe { PROCESSOR.current().unwrap() };
//         if let Ok(signal_no) = SignalNo::try_from(signum) {
//             if signal_no == SignalNo::ERR {
//                 return -1;
//             }
//             // 如果需要返回原来的处理函数，则从信号模块中获取
//             if old_action as usize != 0 {
//                 if let Some(mut ptr) = current
//                     .address_space
//                     .translate(VAddr::new(old_action), WRITEABLE)
//                 {
//                     if let Some(signal_action) = current.signal.get_action_ref(signal_no) {
//                         *unsafe { ptr.as_mut() } = signal_action;
//                     } else {
//                         return -1;
//                     }
//                 } else {
//                     // 如果返回了 None，说明 signal_no 无效
//                     return -1;
//                 }
//             }
//             // 如果需要设置新的处理函数，则设置到信号模块中
//             if action as usize != 0 {
//                 if let Some(ptr) = current
//                     .address_space
//                     .translate(VAddr::new(action), READABLE)
//                 {
//                     // 如果返回了 false，说明 signal_no 无效
//                     if !current
//                         .signal
//                         .set_action(signal_no, &unsafe { *ptr.as_ptr() })
//                     {
//                         return -1;
//                     }
//                 } else {
//                     return -1;
//                 }
//             }
//             return 0;
//         }
//         -1
//     }

//     fn sigprocmask(&self, _caller: Caller, mask: usize) -> isize {
//         let current = unsafe { PROCESSOR.current().unwrap() };
//         current.signal.update_mask(mask) as isize
//     }

//     fn sigreturn(&self, _caller: Caller) -> isize {
//         let current = unsafe { PROCESSOR.current().unwrap() };
//         // 如成功，则需要修改当前用户程序的 LocalContext
//         if current.signal.sig_return(&mut current.context.context) {
//             0
//         } else {
//             -1
//         }
//     }
// }



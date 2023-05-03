//! 一种信号模块的实现

#![no_std]

extern crate alloc;
use alloc::boxed::Box;
#[cfg(feature = "user")]
use signal::LocalContext;
#[cfg(feature = "kernel")]
use kernel_context::LocalContext;
use signal::{Signal, SignalAction, SignalNo, SignalResult, MAX_SIG};

mod default_action;
use default_action::DefaultAction;
mod signal_set;
use signal_set::SignalSet;


/// 正在处理的信号
pub enum HandlingSignal {
    Frozen,                   // 是内核信号，需要暂停当前进程
    UserSignal(LocalContext), // 是用户信号，需要保存之前的用户栈
}

/// 管理一个进程中的信号
pub struct SignalImpl {
    /// 已收到的信号
    pub received: SignalSet,
    /// 屏蔽的信号掩码
    pub mask: SignalSet,
    /// 在信号处理函数中，保存之前的用户栈
    pub handling: Option<HandlingSignal>,
    /// 当前任务的信号处理函数集
    pub actions: [Option<SignalAction>; MAX_SIG + 1],
}

impl SignalImpl {
    pub fn new() -> Self {
        Self {
            received: SignalSet::empty(),
            mask: SignalSet::empty(),
            handling: None,
            actions: [None; MAX_SIG + 1],
        }
    }
}

impl SignalImpl {
    /// 获取一个没有被 mask 屏蔽的信号，并从已收到的信号集合中删除它。如果没有这样的信号，则返回空
    fn fetch_signal(&mut self) -> Option<SignalNo> {
        // 在已收到的信号中，寻找一个没有被 mask 屏蔽的信号
        self.received.find_first_one(self.mask).map(|num| {
            self.received.remove_bit(num);
            num.into()
        })
    }

    /// 检查是否收到一个信号，如果是，则接收并删除它
    fn fetch_and_remove(&mut self, signal_no: SignalNo) -> bool {
        if self.received.contain_bit(signal_no as usize)
            && !self.mask.contain_bit(signal_no as usize)
        {
            self.received.remove_bit(signal_no as usize);
            true
        } else {
            false
        }
    }
}

impl Signal for SignalImpl {
    fn from_fork(&mut self) -> Box<dyn Signal> {
        Box::new(Self {
            received: SignalSet::empty(),
            mask: self.mask,
            handling: None,
            actions: {
                let mut actions = [None; MAX_SIG + 1];
                actions.copy_from_slice(&self.actions);
                actions
            },
        })
    }

    fn clear(&mut self) {
        for action in &mut self.actions {
            action.take();
        }
    }

    /// 添加一个信号
    fn add_signal(&mut self, signal: SignalNo) {
        self.received.add_bit(signal as usize)
    }

    /// 是否当前正在处理信号
    fn is_handling_signal(&self) -> bool {
        self.handling.is_some()
    }

    /// 设置一个信号处理函数。`sys_sigaction` 会使用
    fn set_action(&mut self, signum: SignalNo, action: &SignalAction) -> bool {
        if signum == SignalNo::SIGKILL || signum == SignalNo::SIGSTOP {
            false
        } else {
            self.actions[signum as usize] = Some(*action);
            true
        }
    }

    /// 获取一个信号处理函数的值。`sys_sigaction` 会使用
    fn get_action_ref(&self, signum: SignalNo) -> Option<SignalAction> {
        if signum == SignalNo::SIGKILL || signum == SignalNo::SIGSTOP {
            None
        } else {
            Some(self.actions[signum as usize].unwrap_or(SignalAction::default()))
        }
    }

    /// 设置信号掩码，并获取旧的信号掩码，`sys_procmask` 会使用
    fn update_mask(&mut self, mask: usize) -> usize {
        self.mask.set_new(mask.into())
    }

    fn handle_signals(&mut self, current_context: &mut LocalContext) -> SignalResult {
        if self.is_handling_signal() {
            match self.handling.as_ref().unwrap() {
                // 如果当前正在暂停状态
                HandlingSignal::Frozen => {
                    // 则检查是否收到 SIGCONT，如果收到则当前任务需要从暂停状态中恢复
                    if self.fetch_and_remove(SignalNo::SIGCONT) {
                        self.handling.take();
                        SignalResult::Handled
                    } else {
                        // 否则，继续暂停
                        SignalResult::ProcessSuspended
                    }
                } // 其他情况下，需要等待当前信号处理结束
                _ => SignalResult::IsHandlingSignal,
            }
        } else if let Some(signal) = self.fetch_signal() {
            match signal {
                // SIGKILL 信号不能被捕获或忽略
                SignalNo::SIGKILL => SignalResult::ProcessKilled(-(signal as i32)),
                SignalNo::SIGSTOP => {
                    self.handling = Some(HandlingSignal::Frozen);
                    SignalResult::ProcessSuspended
                }
                _ => {
                    if let Some(action) = self.actions[signal as usize] {
                        // 如果用户给定了处理方式，则按照 SignalAction 中的描述处理
                        // 保存原来用户程序的上下文信息
                        self.handling = Some(HandlingSignal::UserSignal(current_context.clone()));
                        // 修改返回后的 pc 值为 handler，修改 a0 为信号编号
                        //println!("handle pre {:x}, after {:x}", current_context.pc(), action.handler);
                        *current_context.pc_mut() = action.handler;
                        *current_context.a_mut(0) = signal as usize;
                        SignalResult::Handled
                    } else {
                        // 否则，使用自定义的 DefaultAction 类来处理
                        // 然后再转换成 SignalResult
                        DefaultAction::from(signal).into()
                    }
                }
            }
        } else {
            SignalResult::NoSignal
        }
    }

    fn sig_return(&mut self, current_context: &mut LocalContext) -> bool {
        let handling_signal = self.handling.take();
        match handling_signal {
            Some(HandlingSignal::UserSignal(old_ctx)) => {
                //println!("return to {:x} a0 {}", old_ctx.pc(), old_ctx.a(0));
                *current_context = old_ctx;
                true
            }
            // 如果当前在处理内核信号，或者没有在处理信号，也就谈不上“返回”了
            _ => {
                self.handling = handling_signal;
                false
            }
        }
    }
}


# [cfg(test)]
mod tests{
    use signal::Signal;

    use crate::{SignalImpl, SignalSet};

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



    #[test]
    fn test_signal_impl() {
        let mut sig1 = SignalImpl::new();
        let sig2 = SignalImpl::new();
        let fetch1 = (&mut sig1).fetch_signal();
        assert_eq!(None, fetch1);
        let fetch2 = (&mut sig1).fetch_and_remove(signal::SignalNo::ERR);
        assert_eq!(false, fetch2);
        (&mut sig1).from_fork();
        (&mut sig1).clear();
        (&mut sig1).add_signal(signal::SignalNo::SIGABRT);
        let hand1 = (& sig2).is_handling_signal();
        assert_eq!(false, hand1);
        (& sig2).get_action_ref(signal::SignalNo::SIGABRT);
    }

    #[test]
    fn test_default_action() {
        
    }

    #[test]
    fn test_signal_set() {
        let value = 1;
        let mut sigset1 = SignalSet::empty();
        SignalSet::new(value);
        (&mut sigset1).reset(value);
        (&mut sigset1).clear();
    }
}
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
    /// 创建一个新的信号管理器
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

    use crate::{SignalImpl, SignalSet, DefaultAction};
    use crate::LocalContext;
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

        let mask1 = (&mut sig1).update_mask(0001);
        assert_eq!(0, mask1);
        let mask1 = (&mut sig1).update_mask(0002);
        assert_eq!(1, mask1);

        let mut local1 = LocalContext::empty();
        let _result1 = (&mut sig1).handle_signals(&mut local1);

    }

    #[test]
    fn test_default_action() {
        let default1 = DefaultAction::Ignore;
        let default2 = DefaultAction::from(signal::SignalNo::SIGABRT);
        let default3 = DefaultAction::from(signal::SignalNo::SIGCHLD);
        let default4 = DefaultAction::from(signal::SignalNo::SIGURG);
        assert_eq!(default3, default1);
        assert_eq!(default4, default1);
        assert_eq!(default2, DefaultAction::Terminate(-6));
        // DefaultAction::into(default1);
    }

    #[test]
    fn test_signal_set() {
        let value = 2;
        let kth1 = 1;
        let kth2 = 10;
        //创建字符数组sigset
        let mut sigset1 = SignalSet::empty();
        let sigset2 = SignalSet::new(value);
        //直接暴力写入 SignalSet，sigset1.0 = value
        (&mut sigset1).reset(value);
        assert_eq!(sigset1.0, 2);
        //清空 SignalSet
        (&mut sigset1).clear();
        assert_eq!(sigset1.0, 0);
        //新增一个 bit
        (&mut sigset1).add_bit(kth1);
        assert_eq!(sigset1.0, 2);
        //删除一个 bit
        (&mut sigset1).remove_bit(kth1);
        assert_eq!(sigset1.0, 0);
        //取交集
        (&mut sigset1).get_union(sigset2);
        assert_eq!(sigset1.0, 2);
        //取差集，即去掉 set 中的内容
        (&mut sigset1).get_difference(sigset2);
        assert_eq!(sigset1.0, 0);
        //直接设置为新值，并返回旧的值
        let old = (&mut sigset1).set_new(sigset2);
        assert_eq!(sigset1.0, 2);
        assert_eq!(old, 0);

        //是否包含第 k 个 bit
        let con1 = (& sigset2).contain_bit(kth1);
        assert_eq!(con1, true);
        let con2 = (& sigset2).contain_bit(kth2);
        assert_eq!(con2, false);
        //获取后缀0个数，可以用来寻找最小的1
        let little = sigset2.get_trailing_zeros();
        assert_eq!(little, 1);
        // //寻找不在mask中的最小的 1 的位置，如果有，返回其位置，如没有则返回 None。
        // sigset2.find_first_one();
    }
}
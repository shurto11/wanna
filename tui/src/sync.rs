//! サーバーとの同期。UI を止めないよう別スレッドで動かす。
//!
//! - worker: outbox を順に送り、続けて差分を取得する
//! - listener: SSE を購読し、rev が進んだら知らせる（切れたら再接続）

use crate::store::Op;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;
use wanna_core::client::{Client, Rejected};
use wanna_core::SyncResponse;

pub enum ToWorker {
    Sync { since: Option<i64>, ops: Vec<(i64, Op)> },
}

pub enum FromSync {
    Done {
        /// 送信できた outbox の seq
        acked: Vec<i64>,
        /// サーバーに拒否された（再送しても無駄な）seq
        rejected: Vec<i64>,
        pulled: Option<SyncResponse>,
        error: Option<String>,
    },
    /// SSE で rev が進んだ
    Remote(i64),
    /// SSE が切れた（再接続を待つ）
    Disconnected,
}

pub fn spawn(client: Client, rx: Receiver<ToWorker>, tx: Sender<FromSync>) {
    let listener_client = client.clone();
    let listener_tx = tx.clone();
    thread::spawn(move || worker(client, rx, tx));
    thread::spawn(move || loop {
        let tx = listener_tx.clone();
        let _ = listener_client.listen(|rev| {
            let _ = tx.send(FromSync::Remote(rev));
        });
        if listener_tx.send(FromSync::Disconnected).is_err() {
            return; // UI が終了した
        }
        thread::sleep(Duration::from_secs(5));
    });
}

fn worker(client: Client, rx: Receiver<ToWorker>, tx: Sender<FromSync>) {
    for ToWorker::Sync { since, ops } in rx {
        let mut acked = Vec::new();
        let mut rejected = Vec::new();
        let mut error = None;
        for (seq, op) in ops {
            let res = match &op {
                Op::Create { want } => client.create(want).map(drop),
                Op::Patch { id, patch } => client.patch(id, patch).map(drop),
                Op::Delete { id } => client.delete(id),
            };
            match res {
                Ok(()) => acked.push(seq),
                Err(e) if e.is::<Rejected>() => rejected.push(seq),
                Err(e) => {
                    // 到達不能など。順序を守るため、ここで打ち切って次回に再送する
                    error = Some(format!("{e:#}"));
                    break;
                }
            }
        }
        let pulled = if error.is_none() {
            match client.sync(since) {
                Ok(r) => Some(r),
                Err(e) => {
                    error = Some(format!("{e:#}"));
                    None
                }
            }
        } else {
            None
        };
        if tx.send(FromSync::Done { acked, rejected, pulled, error }).is_err() {
            return;
        }
    }
}

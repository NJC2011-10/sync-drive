use futures::executor::block_on;
use sync_drive::{StackRuntime, blocking};

// 无参、无返回值
#[blocking]
async fn noop() {}

// 参数 + 返回值
#[blocking]
async fn add(a: i32, b: i32) -> i32 {
	a + b
}

// 返回 Result
#[blocking]
async fn parse_num(s: String) -> Result<i32, std::num::ParseIntError> {
	s.parse::<i32>()
}

// 属性保留
#[blocking]
#[allow(dead_code)]
async fn with_attr() -> u32 {
	1
}

// pub 可见性不被破坏
#[blocking]
pub async fn pub_fn(x: u32) -> u32 {
	x + 1
}

// 内部有真正的 await，验证宏确实在驱动异步
#[blocking]
async fn sleep_ms(ms: u64) -> u64 {
	tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
	ms
}

#[test]
fn macro_noop() {
	let rt = StackRuntime::new(|ctx| noop(&ctx)).unwrap();
	block_on(rt);
}

#[test]
fn macro_add() {
	let rt = StackRuntime::new(|ctx| add(&ctx, 2, 3)).unwrap();
	assert_eq!(block_on(rt), 5);
}

#[test]
fn macro_result_ok() {
	let rt = StackRuntime::new(|ctx| parse_num(&ctx, "42".into())).unwrap();
	assert_eq!(block_on(rt), Ok(42));
}

#[test]
fn macro_result_err() {
	let rt = StackRuntime::new(|ctx| parse_num(&ctx, "nope".into())).unwrap();
	assert!(block_on(rt).is_err());
}

#[test]
fn macro_result_question_mark() {
	let rt = StackRuntime::new(|ctx| -> Result<i32, std::num::ParseIntError> {
		let a = parse_num(&ctx, "10".into())?;
		let b = parse_num(&ctx, "20".into())?;
		Ok(a + b)
	})
	.unwrap();
	assert_eq!(block_on(rt), Ok(30));
}

#[test]
fn macro_with_attr_and_pub() {
	let rt = StackRuntime::new(|ctx| (with_attr(&ctx), pub_fn(&ctx, 41))).unwrap();
	assert_eq!(block_on(rt), (1, 42));
}

#[test]
fn macro_with_real_await() {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_time()
		.build()
		.unwrap();
	rt.block_on(async {
		let task = StackRuntime::new(|ctx| sleep_ms(&ctx, 10)).unwrap();
		assert_eq!(task.await, 10);
	});
}

struct Counter {
	n: u32,
}

impl Counter {
	#[blocking]
	async fn bump(&mut self) -> u32 {
		self.n += 1;
		self.n
	}
}

#[test]
fn macro_on_method() {
	let rt = StackRuntime::new(|ctx| {
		let mut c = Counter { n: 0 };
		c.bump(&ctx);
		c.bump(&ctx)
	})
	.unwrap();
	assert_eq!(block_on(rt), 2);
}

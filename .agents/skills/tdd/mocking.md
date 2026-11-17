# 何时使用 Mock

仅在**系统边界**处使用 mock：

- 外部 API（支付、邮件等）
- 数据库（有时——更优先使用测试数据库）
- 时间 / 随机数
- 文件系统（有时）

不要 mock：

- 你自己的类 / 模块
- 内部协作者
- 任何受你控制的代码

## 面向可 Mock 性设计

在系统边界处，设计易于 mock 的接口：

**1. 使用依赖注入**

把外部依赖作为参数传入，而不是在内部直接创建：

```typescript
// 易于 mock
function processPayment(order, paymentClient) {
  return paymentClient.charge(order.total);
}

// 难以 mock
function processPayment(order) {
  const client = new StripeClient(process.env.STRIPE_KEY);
  return client.charge(order.total);
}
```

**2. 优先使用 SDK 风格接口，而非通用 fetcher**

为每个外部操作创建专用函数，而不是一个塞满条件判断的通用函数：

```typescript
// 好：每个函数都能独立 mock
const api = {
  getUser: (id) => fetch(`/users/${id}`),
  getOrders: (userId) => fetch(`/users/${userId}/orders`),
  createOrder: (data) => fetch('/orders', { method: 'POST', body: data }),
};

// 差：mock 时不得不在里面写一堆条件分支
const api = {
  fetch: (endpoint, options) => fetch(endpoint, options),
};
```

SDK 风格的好处：
- 每个 mock 只返回一种确定形状
- 测试夹具里不需要条件分支逻辑
- 一眼看出测试覆盖了哪些端点
- 每个端点各自具备类型安全
# 好测试与坏测试

## 好测试

**集成风格**：通过真实接口测试，而不是 mock 内部零件。

```typescript
// 好：测试可观察的行为
test("user can checkout with valid cart", async () => {
  const cart = createCart();
  cart.add(product);
  const result = await checkout(cart, paymentMethod);
  expect(result.status).toBe("confirmed");
});
```

特征：

- 测试用户 / 调用方真正关心的行为
- 只使用公开 API
- 内部重构后依然通过
- 描述 **做什么（WHAT）**，而不是 **怎么做（HOW）**
- 每个测试一个逻辑断言

## 坏测试

**实现细节测试**：与内部结构紧耦合。

```typescript
// 差：测试实现细节
test("checkout calls paymentService.process", async () => {
  const mockPayment = jest.mock(paymentService);
  await checkout(cart, payment);
  expect(mockPayment.process).toHaveBeenCalledWith(cart.total);
});
```

危险信号：

- mock 内部协作者
- 测试私有方法
- 断言调用次数 / 调用顺序
- 行为没变，一重构测试就挂
- 测试名称描述“怎么实现”而不是“实现了什么”
- 绕开接口、通过外部旁路验证结果

```typescript
// 差：绕开接口直接查库验证
test("createUser saves to database", async () => {
  await createUser({ name: "Alice" });
  const row = await db.query("SELECT * FROM users WHERE name = ?", ["Alice"]);
  expect(row).toBeDefined();
});

// 好：通过公开接口验证
test("createUser makes user retrievable", async () => {
  const user = await createUser({ name: "Alice" });
  const retrieved = await getUser(user.id);
  expect(retrieved.name).toBe("Alice");
});
```

**同义反复测试（Tautological tests）**：期望值用与实现相同的方式重算一遍，导致测试在构造上永远通过。

```typescript
// 差：用和业务代码一样的方式重算期望值
test("calculateTotal sums line items", () => {
  const items = [{ price: 10 }, { price: 5 }];
  const expected = items.reduce((sum, i) => sum + i.price, 0);
  expect(calculateTotal(items)).toBe(expected);
});

// 好：期望值来自独立的已知字面量
test("calculateTotal sums line items", () => {
  expect(calculateTotal([{ price: 10 }, { price: 5 }])).toBe(15);
});
```
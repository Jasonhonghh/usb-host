# 设计思路

## 异步模型

基于Ring的无锁设计，每个Ring元素TRB都是一个异步任务，保存异步状态，`future`通过查询`ring`来获取异步结果。

未使用特定`Executor`，且不包含任务队列，因此可以支持任意`Executor`。

![流程图](异步请求.drawio.png)

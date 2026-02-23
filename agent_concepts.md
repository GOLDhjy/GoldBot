# Agent 核心概念

## ReAct 是什么

ReAct 是 AI agent 设计中的一个重要模式，是 **Rea**soning（推理）和 **Act**ing（行动）的组合。

### 核心思想

传统 agent 只是简单地将输入映射到行动。ReAct agent 在执行行动之前会进行**显式的推理**，形成"思考→行动→观察→再思考"的循环。

### 工作循环

```
Thought（思考）: 我需要完成什么任务？当前状态是什么？
    ↓
Action（行动）: 选择一个工具或函数来执行
    ↓
Observation（观察）: 获取行动的结果
    ↓
Thought（思考）: 结果如何？下一步该做什么？
    ↓
...循环直到任务完成
```

### 实际例子

```
Thought: 用户想知道天气，但我不知道他的位置
Action: ask_location()
Observation: 返回 "北京"
Thought: 现在知道位置了，可以查询天气
Action: get_weather("北京")
Observation: 返回 "晴天，22°C"
Thought: 任务完成，给用户回复
Action: respond("北京今天晴天，22°C")
```

### 为什么有效

1. **可解释性** - 每一步思考过程都可见
2. **纠错能力** - 行动失败时可以通过推理重新规划
3. **复杂任务分解** - 可以把大任务拆成多步

这个模式在 LangChain、AutoGPT 等 agent 框架中被广泛使用。

---

## LangChain 是什么

LangChain 是一个用于开发由大语言模型（LLM）驱动的应用程序的框架，最初用 Python 构建，后来也支持 JavaScript/TypeScript。

### 核心概念

LangChain 把 LLM 应用开发的常见模式抽象成了可复用的组件：

#### 1. Chains（链）
将多个组件串联起来，形成处理流程：
```python
# 简单链：Prompt → LLM → Output
chain = LLMChain(llm=llm, prompt=prompt)

# 复杂链：可以串联多个步骤
chain = SimpleSequentialChain(
    chains=[chain1, chain2, chain3]
)
```

#### 2. Agents（代理）
就是前面说的 ReAct 模式的实现：
```python
agent = Agent(
    llm=llm,
    tools=[search_tool, calculator_tool, database_tool]
)
# agent 会自主决定使用哪个工具
```

#### 3. Tools（工具）
给 LLM 提供的外部能力，比如：
- 网络搜索
- 代码执行
- 数据库查询
- API 调用

#### 4. Memory（记忆）
让对话记住之前的内容：
- 短期记忆（当前会话）
- 长期记忆（跨会话持久化）

#### 5. Prompts（提示词模板）
结构化管理提示词：
```python
template = "你是一个{name}，请回答：{question}"
prompt = PromptTemplate(
    template=template,
    input_variables=["name", "question"]
)
```

#### 6. Retrieval（检索）
RAG（检索增强生成）相关组件，用于构建知识库问答系统。

### 为什么需要它

直接调用 OpenAI API 很简单，但构建实际应用时你会遇到：
- 需要串联多个 LLM 调用
- 需要连接外部数据源
- 需要记忆对话历史
- 需要让 LLM 使用工具

LangChain 把这些常见需求都标准化了。

### 现状

LangChain 在 2023 年非常火，但也有一些批评：
- 抽象层次过多，复杂度高
- API 变化频繁
- 学习曲线陡峭

现在很多人也倾向于用更轻量的方案（如 LlamaIndex、直接调用 API 等），取决于具体需求。

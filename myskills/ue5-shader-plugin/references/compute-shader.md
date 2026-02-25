# Compute Shader（RDG 优先）参考

用于插件内通过 RDG Dispatch 的 Compute Shader 骨架。

## 1. Shader 类模板

```cpp
class FMyExampleCS : public FGlobalShader
{
	DECLARE_GLOBAL_SHADER(FMyExampleCS);
	SHADER_USE_PARAMETER_STRUCT(FMyExampleCS, FGlobalShader);

public:
	static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
	{
		return IsFeatureLevelSupported(Parameters.Platform, ERHIFeatureLevel::SM5);
	}

	static void ModifyCompilationEnvironment(const FGlobalShaderPermutationParameters& Parameters, FShaderCompilerEnvironment& OutEnvironment)
	{
		FGlobalShader::ModifyCompilationEnvironment(Parameters, OutEnvironment);
		OutEnvironment.SetDefine(TEXT("THREADGROUP_SIZE_X"), 8);
		OutEnvironment.SetDefine(TEXT("THREADGROUP_SIZE_Y"), 8);
	}

	BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
		SHADER_PARAMETER(FIntPoint, OutputSize)
		SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, OutputTexture)
	END_SHADER_PARAMETER_STRUCT()
};

IMPLEMENT_GLOBAL_SHADER(FMyExampleCS, "/Plugin/MyPlugin/Private/MyExampleCS.usf", "MainCS", SF_Compute);
```

## 2. 最小 `.usf` 模板

File: `Shaders/Private/MyExampleCS.usf`

```hlsl
#ifndef THREADGROUP_SIZE_X
#define THREADGROUP_SIZE_X 8
#endif

#ifndef THREADGROUP_SIZE_Y
#define THREADGROUP_SIZE_Y 8
#endif

RWTexture2D<float4> OutputTexture;
int2 OutputSize;

[numthreads(THREADGROUP_SIZE_X, THREADGROUP_SIZE_Y, 1)]
void MainCS(uint3 DispatchThreadId : SV_DispatchThreadID)
{
	if (DispatchThreadId.x >= OutputSize.x || DispatchThreadId.y >= OutputSize.y)
	{
		return;
	}

	float2 uv = (DispatchThreadId.xy + 0.5) / float2(OutputSize);
	OutputTexture[DispatchThreadId.xy] = float4(uv, 0.0, 1.0);
}
```

## 3. RDG Dispatch 模板

```cpp
FRDGTextureDesc OutputDesc = FRDGTextureDesc::Create2D(
	FIntPoint(SizeX, SizeY),
	PF_A32B32G32R32F,
	FClearValueBinding::Black,
	TexCreate_ShaderResource | TexCreate_UAV);

FRDGTextureRef OutputTexture = GraphBuilder.CreateTexture(OutputDesc, TEXT("MyPlugin.ExampleOutput"));

FMyExampleCS::FParameters* PassParameters = GraphBuilder.AllocParameters<FMyExampleCS::FParameters>();
PassParameters->OutputSize = FIntPoint(SizeX, SizeY);
PassParameters->OutputTexture = GraphBuilder.CreateUAV(OutputTexture);

TShaderMapRef<FMyExampleCS> ComputeShader(GetGlobalShaderMap(GMaxRHIFeatureLevel));
const FIntVector GroupCount = FComputeShaderUtils::GetGroupCount(
	FIntPoint(SizeX, SizeY),
	FIntPoint(8, 8));

FComputeShaderUtils::AddPass(
	GraphBuilder,
	RDG_EVENT_NAME("MyPlugin::MyExampleCS"),
	ComputeShader,
	PassParameters,
	GroupCount);
```

Notes:

1. 优先使用 `FRDGTextureRef` / `FRDGBufferRef` 和 `GraphBuilder.CreateUAV(...)`
2. Compute Dispatch 应该放在渲染线程上下文或 RDG Pass 构建流程中
3. 需要 CPU Readback 时，先找本地同版本示例再写

## 4. CPU Readback 说明（可选）

CPU Readback API 版本差异较大，按这个顺序做：

1. 搜本地引擎/插件里 `FRHIGPUBufferReadback` / `FRHIGPUTextureReadback`
2. 复用同版本的 enqueue、fence/poll、copy 流程
3. 不要在没搞清资源生命周期前混用 RDG extraction 和立即 RHI readback

## 5. 常见错误

1. C++ 和 `.usf` 里的线程组宏不一致
2. Output UAV 格式和 Shader 输出类型不匹配
3. GroupCount 按字节数算了，导致 dispatch 维度错误
4. 在 Game Thread 上直接 dispatch，而不是在 Render Thread/RDG 上下文执行

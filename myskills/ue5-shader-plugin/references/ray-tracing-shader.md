# Ray Tracing Shader（RayGen 优先）参考

用于插件内光追 Shader 脚手架。先做最小可工作的 RayGen，再扩展 Hit/Miss/更复杂流程。

这是三类里版本差异最大的路径。优先使用同分支本地示例，不要直接照抄社区教程。

## 1. 宏保护与编译门控

光追代码必须带宏保护：

```cpp
#if RHI_RAYTRACING
// Ray tracing shader declarations and dispatch code
#endif
```

在 Shader 类 `ShouldCompilePermutation(...)` 中，按项目/平台能力门控：

```cpp
static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
{
	return ShouldCompileRayTracingShadersForProject(Parameters.Platform);
}
```

## 2. RayGen Shader 类模板（常见模式）

优先参考本仓库已有实现。`PRTGI` 已在 `Source/PRTGI/Private/PRTDistantTextureComponent.cpp` 中有 `SF_RayGen` 示例。

```cpp
#if RHI_RAYTRACING
class FMyRayGenRGS : public FGlobalShader
{
	DECLARE_GLOBAL_SHADER(FMyRayGenRGS);
	SHADER_USE_ROOT_PARAMETER_STRUCT(FMyRayGenRGS, FGlobalShader);

public:
	static bool ShouldCompilePermutation(const FGlobalShaderPermutationParameters& Parameters)
	{
		return ShouldCompileRayTracingShadersForProject(Parameters.Platform);
	}

	static ERayTracingPayloadType GetRayTracingPayloadType(const int32 PermutationId)
	{
		return ERayTracingPayloadType::RayTracingMaterial;
	}

	static const FShaderBindingLayout* GetShaderBindingLayout(const FShaderPermutationParameters& Parameters)
	{
		return RayTracing::GetShaderBindingLayout(Parameters.Platform);
	}

	BEGIN_SHADER_PARAMETER_STRUCT(FParameters, )
		SHADER_PARAMETER_STRUCT_REF(FViewUniformShaderParameters, ViewUniformBuffer)
		SHADER_PARAMETER_RDG_BUFFER_SRV(RaytracingAccelerationStructure, TLAS)
		SHADER_PARAMETER_RDG_TEXTURE_UAV(RWTexture2D<float4>, OutputTexture)
	END_SHADER_PARAMETER_STRUCT()
};

IMPLEMENT_GLOBAL_SHADER(FMyRayGenRGS, "/Plugin/MyPlugin/Private/MyRayTracing.usf", "MyRayGen", SF_RayGen);
#endif
```

Notes:

1. 光追 Global Shader 常见写法是 `SHADER_USE_ROOT_PARAMETER_STRUCT`
2. TLAS 参数宏/类型在不同 UE 版本可能变化，必须对照本地同版本代码
3. 有些项目会使用自定义 payload type，需要和本地渲染路径保持一致

## 3. 最小 `.usf` RayGen 骨架

File: `Shaders/Private/MyRayTracing.usf`

```hlsl
[shader("raygeneration")]
void MyRayGen()
{
	// Minimal raygen entry point.
	// Fill in ray setup, trace call, and output writes after the pipeline path compiles.
}
```

先做“仅能编译”的 RayGen 骨架。等注册和 dispatch 路径打通后，再补 trace 逻辑。

## 4. Dispatch 路径策略（版本安全）

不要把旧教程里的 dispatch API 直接写死。按下面策略做：

1. Search local engine/plugin code for:
   `RayTraceDispatch`, `FRayTracingPipelineStateInitializer`, `GetRayTracingShader()`, `PipelineStateCache::GetAndOrCreateRayTracingPipelineState`
2. Copy the same-version setup pattern for:
   pipeline state creation, raygen table setup, miss/hit tables, shader bindings, and dispatch
3. Replace only:
   shader type, parameters, output target, and dimensions

建议搜索命令：

```powershell
rg -n "FRayTracingPipelineStateInitializer|RayTraceDispatch|SF_RayGen|GetShaderBindingLayout" Source Engine -g "*.cpp"
```

## 5. 项目 / 平台前置条件

在调 Shader 代码前，先确认运行时前置条件：

1. Project uses DX12 (Windows)
2. Hardware ray tracing is enabled in project settings
3. Platform/GPU supports ray tracing
4. Build target and shader platform compile ray tracing permutations

## 6. 常见错误

1. 光追 Shader 能编译，但 dispatch 路径用了不匹配的 payload type
2. TLAS 句柄为空，或在错误的 scene/view 生命周期阶段获取
3. 缺少 `#if RHI_RAYTRACING` 导致非 RT 平台编译失败
4. 社区教程 API 名称与本地 UE 分支不一致
5. 还没验证最小 RayGen 路径就先上完整 hit-group pipeline

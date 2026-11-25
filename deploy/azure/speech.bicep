// Rubato's speech provider on Azure AI Foundry (ADR 0073): one AIServices account with a
// speech-to-text and a text-to-speech deployment, reached over the OpenAI-compatible v1 API.
// Applied by hand (deploy/README.md, "Speech on Azure AI Foundry"); checked by deploy/check.ts.
// The key never leaves Azure through this file: no output reads it.

targetScope = 'resourceGroup'

// East US 2 is the one region with both current speech models and the quota for them; the extra
// ~0.1 s round trip is small next to the agent's turn.
@description('Region of the account')
param location string = 'eastus2'

@description('Account name, also its subdomain: https://<name>.openai.azure.com')
param accountName string = 'rubato-speech'

param tags object = {
  Application: 'Rubato'
  ManagedBy: 'Bicep'
}

// Deployment name = the model field Rubato sends (RUBATO_STT_MODEL, RUBATO_TTS_MODEL).
var deployments = [
  {
    name: 'gpt-4o-transcribe'
    model: 'gpt-4o-transcribe'
    version: '2025-03-20'
    sku: 'GlobalStandard' // processed wherever Azure has capacity (chosen 2026-11-25)
    capacity: 50 // thousand tokens a minute, of the region's 400
  }
  {
    name: 'gpt-4o-mini-tts'
    model: 'gpt-4o-mini-tts'
    version: '2025-12-15'
    sku: 'GlobalStandard'
    capacity: 50 // of the region's 600
  }
]

resource account 'Microsoft.CognitiveServices/accounts@2024-10-01' = {
  name: accountName
  location: location
  tags: tags
  kind: 'AIServices'
  sku: {
    name: 'S0'
  }
  properties: {
    customSubDomainName: accountName
    publicNetworkAccess: 'Enabled'
    disableLocalAuth: false // Rubato authenticates with the account key
  }
}

@batchSize(1)
resource deployment 'Microsoft.CognitiveServices/accounts/deployments@2024-10-01' = [
  for d in deployments: {
    parent: account
    name: d.name
    sku: {
      name: d.sku
      capacity: d.capacity
    }
    properties: {
      model: {
        format: 'OpenAI'
        name: d.model
        version: d.version
      }
    }
  }
]

@description('RUBATO_STT_URL and RUBATO_TTS_URL')
output endpoint string = 'https://${accountName}.openai.azure.com/openai/v1'

@description('RUBATO_STT_MODEL and RUBATO_TTS_MODEL')
output models array = [for d in deployments: d.name]

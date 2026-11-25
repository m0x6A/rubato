// Must fail: key auth off, a model on `latest`, and the key read into an output.
resource account 'Microsoft.CognitiveServices/accounts@2024-10-01' = {
  name: 'rubato-speech'
  location: 'swedencentral'
  kind: 'AIServices'
  sku: {
    name: 'S0'
  }
  properties: {
    disableLocalAuth: true
  }
}

resource tts 'Microsoft.CognitiveServices/accounts/deployments@2024-10-01' = {
  parent: account
  name: 'tts'
  properties: {
    model: {
      format: 'OpenAI'
      name: 'tts'
      version: 'latest'
    }
  }
}

output key string = account.listKeys().key1

from enum import StrEnum

class SpeechAccountKind(StrEnum):
    AWS_POLLY = "aws_polly"
    AZURE = "azure"
    DEEPGRAM = "deepgram"
    ELEVENLABS = "elevenlabs"
    GOOGLE_CLOUD = "google_cloud"
    OPENAI = "openai"

    def __str__(self) -> str:
        return str(self.value)

from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field
import json
from .. import types

from ..types import UNSET, Unset

from ..types import File, FileTypes
from ..types import UNSET, Unset
from io import BytesIO
from typing import cast






T = TypeVar("T", bound="MyGreetingUpload")



@_attrs_define
class MyGreetingUpload:
    """ `POST /me/voicemail/greetings`.

        Attributes:
            greeting_type (str): `unavailable`, `busy`, `name` or `temporary`.
            audio (File): The recording: a WAV file.
            expires_in (int | None | Unset): A temporary greeting's lifetime, in hours.
     """

    greeting_type: str
    audio: File
    expires_in: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        greeting_type = self.greeting_type

        audio = self.audio.to_tuple()


        expires_in: int | None | Unset
        if isinstance(self.expires_in, Unset):
            expires_in = UNSET
        else:
            expires_in = self.expires_in


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "greeting_type": greeting_type,
            "audio": audio,
        })
        if expires_in is not UNSET:
            field_dict["expires_in"] = expires_in

        return field_dict


    def to_multipart(self) -> types.RequestFiles:
        files: types.RequestFiles = []

        files.append(("greeting_type", (None, str(self.greeting_type).encode(), "text/plain")))



        files.append(("audio", self.audio.to_tuple()))



        if not isinstance(self.expires_in, Unset):
            if isinstance(self.expires_in, int):

                files.append(("expires_in", (None, str(self.expires_in).encode(), "text/plain")))
            else:
                files.append(("expires_in", (None, str(self.expires_in).encode(), "text/plain")))



        for prop_name, prop in self.additional_properties.items():
            files.append((prop_name, (None, str(prop).encode(), "text/plain")))



        return files


    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        greeting_type = d.pop("greeting_type")

        audio = File(
             payload = BytesIO(d.pop("audio"))
        )




        def _parse_expires_in(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        expires_in = _parse_expires_in(d.pop("expires_in", UNSET))


        my_greeting_upload = cls(
            greeting_type=greeting_type,
            audio=audio,
            expires_in=expires_in,
        )


        my_greeting_upload.additional_properties = d
        return my_greeting_upload

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties

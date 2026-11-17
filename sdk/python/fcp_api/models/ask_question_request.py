from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="AskQuestionRequest")



@_attrs_define
class AskQuestionRequest:
    """ The body of `POST /me/conferences/live/:room/questions`.

        Attributes:
            call_id (str): The app's call in the room: the `X-FCP-Call-Id` its 200 OK named.
            text (str): The question, at most 500 characters.
            anonymous (bool | Unset): Asked without the asker's name, where the meeting allows it.
     """

    call_id: str
    text: str
    anonymous: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        text = self.text

        anonymous = self.anonymous


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "text": text,
        })
        if anonymous is not UNSET:
            field_dict["anonymous"] = anonymous

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        text = d.pop("text")

        anonymous = d.pop("anonymous", UNSET)

        ask_question_request = cls(
            call_id=call_id,
            text=text,
            anonymous=anonymous,
        )


        ask_question_request.additional_properties = d
        return ask_question_request

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

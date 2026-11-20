from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.question_status import QuestionStatus
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="AttendanceQuestion")



@_attrs_define
class AttendanceQuestion:
    """ A question of the sitting, as kept.

        Attributes:
            id (str):
            participant_id (str):
            text (str):
            status (QuestionStatus): Where a question is.
            published (bool):
            upvotes (int):
            answered_live (bool):
            asked_at (datetime.datetime):
            asker (None | str | Unset):
            answer (None | str | Unset):
     """

    id: str
    participant_id: str
    text: str
    status: QuestionStatus
    published: bool
    upvotes: int
    answered_live: bool
    asked_at: datetime.datetime
    asker: None | str | Unset = UNSET
    answer: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        participant_id = self.participant_id

        text = self.text

        status = self.status.value

        published = self.published

        upvotes = self.upvotes

        answered_live = self.answered_live

        asked_at = self.asked_at.isoformat()

        asker: None | str | Unset
        if isinstance(self.asker, Unset):
            asker = UNSET
        else:
            asker = self.asker

        answer: None | str | Unset
        if isinstance(self.answer, Unset):
            answer = UNSET
        else:
            answer = self.answer


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "participant_id": participant_id,
            "text": text,
            "status": status,
            "published": published,
            "upvotes": upvotes,
            "answered_live": answered_live,
            "asked_at": asked_at,
        })
        if asker is not UNSET:
            field_dict["asker"] = asker
        if answer is not UNSET:
            field_dict["answer"] = answer

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        participant_id = d.pop("participant_id")

        text = d.pop("text")

        status = QuestionStatus(d.pop("status"))




        published = d.pop("published")

        upvotes = d.pop("upvotes")

        answered_live = d.pop("answered_live")

        asked_at = datetime.datetime.fromisoformat(d.pop("asked_at"))




        def _parse_asker(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        asker = _parse_asker(d.pop("asker", UNSET))


        def _parse_answer(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        answer = _parse_answer(d.pop("answer", UNSET))


        attendance_question = cls(
            id=id,
            participant_id=participant_id,
            text=text,
            status=status,
            published=published,
            upvotes=upvotes,
            answered_live=answered_live,
            asked_at=asked_at,
            asker=asker,
            answer=answer,
        )


        attendance_question.additional_properties = d
        return attendance_question

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

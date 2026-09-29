from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SubjectExpansion")



@_attrs_define
class SubjectExpansion:
    """ A subject as the tenant knows them: every spelling that identifies them,
    and the platform objects that are theirs (`docs/COMPLIANCE_FOLLOWUPS.md`
    §6.1). Taken once, when a hold is placed or a request starts, and kept
    with it.

        Attributes:
            identifiers (list[str]): Usernames, extensions, their addresses on the tenant's SIP domains,
                numbers and email addresses.
            user_ids (list[str] | Unset): The platform users among the subjects.
            mailbox_ids (list[str] | Unset): Their mailboxes.
     """

    identifiers: list[str]
    user_ids: list[str] | Unset = UNSET
    mailbox_ids: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        identifiers = self.identifiers



        user_ids: list[str] | Unset = UNSET
        if not isinstance(self.user_ids, Unset):
            user_ids = self.user_ids



        mailbox_ids: list[str] | Unset = UNSET
        if not isinstance(self.mailbox_ids, Unset):
            mailbox_ids = self.mailbox_ids




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "identifiers": identifiers,
        })
        if user_ids is not UNSET:
            field_dict["user_ids"] = user_ids
        if mailbox_ids is not UNSET:
            field_dict["mailbox_ids"] = mailbox_ids

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        identifiers = cast(list[str], d.pop("identifiers"))


        user_ids = cast(list[str], d.pop("user_ids", UNSET))


        mailbox_ids = cast(list[str], d.pop("mailbox_ids", UNSET))


        subject_expansion = cls(
            identifiers=identifiers,
            user_ids=user_ids,
            mailbox_ids=mailbox_ids,
        )


        subject_expansion.additional_properties = d
        return subject_expansion

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
